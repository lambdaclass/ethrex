use crate::{
    EMPTY_TRIE_HASH, Nibbles, Node, TrieDB, TrieError,
    node::{BranchNode, ExtensionNode, LeafNode},
    threadpool::ThreadPool,
};
use crossbeam::channel::{Receiver, Sender, bounded};
use ethereum_types::H256;
use std::{sync::Arc, thread::scope};

/// The elements of the stack represent the branch node that is the parent of the current
/// parent element. When the current parent is no longer valid (is not the parent of
/// the current elements), the stack gets popped and this element becomes the parent
#[derive(Debug, Default, Clone)]
struct StackElement {
    path: Nibbles,
    element: BranchNode,
}

// The large size isn't a performance problem because we use a single instance of this
// struct
#[allow(clippy::large_enum_variant)]
/// This struct handles the current element that the algorithm is processing. The
/// current parent is the parent of this element and the next one in the queue.
/// If that isn't true, we pop the stack and the old parent becomes the new current element
/// This is an enum because the current element can be a leaf or a branch
#[derive(Debug, Clone)]
enum CenterSideElement {
    Branch { node: BranchNode },
    Leaf { value: Vec<u8> },
}

/// The current element and its full path.
#[derive(Debug, Clone)]
struct CenterSide {
    // Full path to the element
    path: Nibbles,
    // Element, can be branch or leaf
    element: CenterSideElement,
}

/// These errors should never happen on a correctly ordered list, but they can happen if
/// the iterator used as input has repeated or out of order values
#[derive(Debug, thiserror::Error)]
pub enum TrieGenerationError {
    #[error("When creating a child node, the nibbles diff was empty. Child Node {0:x?}")]
    IndexNotFound(Nibbles),
    #[error("When popping from the trie stack it was empty. Current position: {0:x?}")]
    TrieStackEmpty(Nibbles),
    #[error(transparent)]
    FlushToDbError(TrieError),
    #[error("When joining the write threads, error")]
    ThreadJoinError(),
}

/// How many nodes we group before sending to write
pub const SIZE_TO_WRITE_DB: u64 = 20_000;
/// How many write buffers we can use at the same time.
/// This number and SIZE_TO_WRITE_DB limits how much memory we use
pub const BUFFER_COUNT: u64 = 32;

impl CenterSide {
    fn from_value(tuple: (H256, Vec<u8>)) -> CenterSide {
        CenterSide {
            path: Nibbles::from_raw(&tuple.0.0, true),
            element: CenterSideElement::Leaf { value: tuple.1 },
        }
    }
    fn from_stack_element(element: StackElement) -> CenterSide {
        CenterSide {
            path: element.path,
            element: CenterSideElement::Branch {
                node: element.element,
            },
        }
    }
}

/// Checks if the stack element is a child node of the element at path `this`
fn is_child(this: &Nibbles, other: &StackElement) -> bool {
    this.count_prefix(&other.path) == other.path.len()
}

/// Creates a parent element that can have as children both the parent and the closest nibbles
/// That parent is created with no children
fn create_parent(current_node: &CenterSide, closest_nibbles: &Nibbles) -> StackElement {
    let new_parent_nibbles = current_node
        .path
        .slice(0, current_node.path.count_prefix(closest_nibbles));
    StackElement {
        path: new_parent_nibbles,
        element: BranchNode {
            choices: BranchNode::EMPTY_CHOICES,
            value: vec![],
        },
    }
}

/// This function modifies a parent element to include the `current_node` element, and
/// then adds the `current_node` to the write queue.
/// When adding the current_node to the write queue we create an extension if needed
fn add_current_to_parent_and_write_queue(
    nodes_to_write: &mut Vec<(Nibbles, Node)>,
    current_node: &CenterSide,
    parent_element: &mut StackElement,
) -> Result<(), TrieGenerationError> {
    let mut nodehash_buffer = Vec::with_capacity(512);
    let mut path = current_node.path.clone();
    path.skip_prefix(&parent_element.path);
    let index = path
        .next()
        .ok_or_else(|| TrieGenerationError::IndexNotFound(current_node.path.clone()))?;
    let top_path = parent_element.path.append_new(index);
    let (target_path, node): (Nibbles, Node) = match &current_node.element {
        CenterSideElement::Branch { node } => {
            if path.is_empty() {
                (top_path, node.clone().into())
            } else {
                let hash = node.compute_hash_no_alloc(&mut nodehash_buffer);
                nodes_to_write.push((current_node.path.clone(), node.clone().into()));
                (
                    top_path,
                    ExtensionNode {
                        prefix: path,
                        child: hash.into(),
                    }
                    .into(),
                )
            }
        }
        CenterSideElement::Leaf { value } => (
            top_path,
            LeafNode {
                partial: path,
                value: value.clone(),
            }
            .into(),
        ),
    };
    parent_element.element.choices[index as usize] =
        node.compute_hash_no_alloc(&mut nodehash_buffer).into();
    nodes_to_write.push((target_path, node));
    Ok(())
}

/// flush_nodes_to_write writes the nodes into the database, and when it's done it
/// returns the vector used to write nodes into the channel for future use
fn flush_nodes_to_write(
    mut nodes_to_write: Vec<(Nibbles, Node)>,
    db: &dyn TrieDB,
    sender: Sender<Vec<(Nibbles, Node)>>,
) -> Result<(), TrieGenerationError> {
    db.put_batch_no_alloc(&nodes_to_write)
        .map_err(TrieGenerationError::FlushToDbError)?;
    nodes_to_write.clear();
    let _ = sender.send(nodes_to_write);
    Ok(())
}

/// Core trie building loop. Processes sorted (H256, Vec<u8>) pairs into a trie structure,
/// writing intermediate nodes to DB via the ThreadPool. Returns the root-level BranchNode
/// (with choices populated) and any unflushed nodes, or None if the iterator was empty.
#[allow(clippy::type_complexity)]
fn trie_from_sorted_accounts_inner<'scope, T>(
    db: &'scope dyn TrieDB,
    data_iter: &mut T,
    scope: Arc<ThreadPool<'scope>>,
    buffer_sender: Sender<Vec<(Nibbles, Node)>>,
    buffer_receiver: Receiver<Vec<(Nibbles, Node)>>,
) -> Result<Option<(BranchNode, Vec<(Nibbles, Node)>)>, TrieGenerationError>
where
    T: Iterator<Item = (H256, Vec<u8>)> + Send,
{
    let Some(initial_value) = data_iter.next() else {
        return Ok(None);
    };
    let mut nodes_to_write: Vec<(Nibbles, Node)> = buffer_receiver
        .recv()
        .expect("This channel shouldn't close");
    let mut trie_stack: Vec<StackElement> = Vec::with_capacity(64);
    let mut current_parent = StackElement::default();
    let mut current_node: CenterSide = CenterSide::from_value(initial_value);
    let mut next_value_opt: Option<(H256, Vec<u8>)> = data_iter.next();

    while let Some(next_value) = next_value_opt {
        if nodes_to_write.len() as u64 > SIZE_TO_WRITE_DB {
            let buffer_sender = buffer_sender.clone();
            scope.execute_priority(Box::new(move || {
                let _ = flush_nodes_to_write(nodes_to_write, db, buffer_sender);
            }));
            nodes_to_write = buffer_receiver
                .recv()
                .expect("This channel shouldn't close");
        }

        let next_value_path = Nibbles::from_bytes(next_value.0.as_bytes());

        while !is_child(&next_value_path, &current_parent) {
            add_current_to_parent_and_write_queue(
                &mut nodes_to_write,
                &current_node,
                &mut current_parent,
            )?;
            let temp = CenterSide::from_stack_element(current_parent);
            current_parent = trie_stack
                .pop()
                .ok_or_else(|| TrieGenerationError::TrieStackEmpty(current_node.path.clone()))?;
            current_node = temp;
        }

        if current_node.path.count_prefix(&current_parent.path)
            == current_node.path.count_prefix(&next_value_path)
        {
            add_current_to_parent_and_write_queue(
                &mut nodes_to_write,
                &current_node,
                &mut current_parent,
            )?;
        } else {
            let mut element = create_parent(&current_node, &next_value_path);
            add_current_to_parent_and_write_queue(
                &mut nodes_to_write,
                &current_node,
                &mut element,
            )?;
            trie_stack.push(current_parent);
            current_parent = element;
        }
        current_node = CenterSide::from_value(next_value);
        next_value_opt = data_iter.next();
    }

    // Empty the stack
    add_current_to_parent_and_write_queue(&mut nodes_to_write, &current_node, &mut current_parent)?;
    while let Some(mut parent_node) = trie_stack.pop() {
        add_current_to_parent_and_write_queue(
            &mut nodes_to_write,
            &CenterSide::from_stack_element(current_parent),
            &mut parent_node,
        )?;
        current_parent = parent_node;
    }

    Ok(Some((current_parent.element, nodes_to_write)))
}

/// Computes the root hash from the root-level BranchNode, handling the single-choice
/// collapse optimization required by the MPT spec. Appends the root node to nodes_to_write.
fn finalize_root(
    root_branch: BranchNode,
    nodes_to_write: &mut Vec<(Nibbles, Node)>,
) -> H256 {
    let mut nodehash_buffer = Vec::with_capacity(512);

    if root_branch
        .choices
        .iter()
        .filter(|choice| choice.is_valid())
        .count()
        == 1
    {
        let (index, child) = root_branch
            .choices
            .into_iter()
            .enumerate()
            .find(|(_, child)| child.is_valid())
            .unwrap();

        let (target_path, node_hash_ref) = nodes_to_write.iter_mut().last().unwrap();
        match node_hash_ref {
            Node::Branch(_) => {
                let node: Node = ExtensionNode {
                    prefix: Nibbles::from_hex(vec![index as u8]),
                    child,
                }
                .into();
                nodes_to_write.push((Nibbles::default(), node));
                nodes_to_write
                    .last()
                    .expect("we just inserted")
                    .1
                    .compute_hash_no_alloc(&mut nodehash_buffer)
                    .finalize()
            }
            Node::Extension(extension_node) => {
                extension_node.prefix.prepend(index as u8);
                // This next works because this target path is always length of 1 element,
                // and we're just removing that one element
                target_path.next();
                extension_node
                    .compute_hash_no_alloc(&mut nodehash_buffer)
                    .finalize()
            }
            Node::Leaf(leaf_node) => {
                leaf_node.partial.prepend(index as u8);
                // This next works because this target path is always length of 1 element,
                // and we're just removing that one element
                target_path.next();
                leaf_node
                    .compute_hash_no_alloc(&mut nodehash_buffer)
                    .finalize()
            }
        }
    } else {
        let node: Node = root_branch.into();
        nodes_to_write.push((Nibbles::default(), node));
        nodes_to_write
            .last()
            .expect("we just inserted")
            .1
            .compute_hash_no_alloc(&mut nodehash_buffer)
            .finalize()
    }
}

/// trie_from_sorted_accounts computes and stores into a db a trie from a sorted
/// iterator of H256 paths and values. This function takes a ThreadPool Arc to send
/// the writing task to be done concurrently.
/// To limit the amount of memory this function can use, we use a crossbeam multiproducer
/// multiconsumer queue, which gives the function a buffer to write nodes into before
/// flushing to the db.
pub fn trie_from_sorted_accounts<'scope, T>(
    db: &'scope dyn TrieDB,
    data_iter: &mut T,
    scope: Arc<ThreadPool<'scope>>,
    buffer_sender: Sender<Vec<(Nibbles, Node)>>,
    buffer_receiver: Receiver<Vec<(Nibbles, Node)>>,
) -> Result<H256, TrieGenerationError>
where
    T: Iterator<Item = (H256, Vec<u8>)> + Send,
{
    let Some((root_branch, mut nodes_to_write)) = trie_from_sorted_accounts_inner(
        db,
        data_iter,
        scope,
        buffer_sender.clone(),
        buffer_receiver,
    )?
    else {
        return Ok(*EMPTY_TRIE_HASH);
    };
    let hash = finalize_root(root_branch, &mut nodes_to_write);
    let _ = flush_nodes_to_write(nodes_to_write, db, buffer_sender);
    Ok(hash)
}

/// Builds the account trie in parallel by partitioning sorted data into 16 buckets
/// (one per first nibble of the key hash) and building subtries concurrently.
/// Each subtrie writes its nodes at correct full paths, then a root BranchNode
/// is created from the 16 subtrie root hashes.
pub fn trie_from_sorted_accounts_parallel<T>(
    db: &dyn TrieDB,
    accounts_iter: &mut T,
) -> Result<H256, TrieGenerationError>
where
    T: Iterator<Item = (H256, Vec<u8>)> + Send,
{
    // Partition data into 16 buckets by first nibble
    let mut buckets: [Vec<(H256, Vec<u8>)>; 16] = Default::default();
    for (key, value) in accounts_iter {
        let first_nibble = (key.0[0] >> 4) as usize;
        buckets[first_nibble].push((key, value));
    }

    let non_empty_count = buckets.iter().filter(|b| !b.is_empty()).count();
    if non_empty_count == 0 {
        return Ok(*EMPTY_TRIE_HASH);
    }
    // Single bucket: fall back to sequential (parallel overhead not worth it)
    if non_empty_count == 1 {
        let items: Vec<_> = buckets.into_iter().flatten().collect();
        return trie_from_sorted_accounts_wrap(db, &mut items.into_iter());
    }

    // Build 16 subtries in parallel using scoped threads + shared ThreadPool for DB writes
    scope(|s| {
        let pool = Arc::new(ThreadPool::new(16, s));

        let handles: Vec<_> = buckets
            .into_iter()
            .enumerate()
            .filter_map(|(nibble, bucket)| {
                if bucket.is_empty() {
                    return None;
                }
                let pool = pool.clone();
                let handle = std::thread::Builder::new()
                    .name(format!("subtrie-{nibble}"))
                    .spawn_scoped(s, move || -> Result<Option<BranchNode>, TrieGenerationError> {
                        let (buf_tx, buf_rx) =
                            bounded::<Vec<(Nibbles, Node)>>(BUFFER_COUNT as usize);
                        for _ in 0..BUFFER_COUNT {
                            let _ = buf_tx.send(Vec::with_capacity(SIZE_TO_WRITE_DB as usize));
                        }
                        let result = trie_from_sorted_accounts_inner(
                            db,
                            &mut bucket.into_iter(),
                            pool,
                            buf_tx.clone(),
                            buf_rx,
                        )?;
                        if let Some((branch, nodes_to_write)) = result {
                            let _ = flush_nodes_to_write(nodes_to_write, db, buf_tx);
                            Ok(Some(branch))
                        } else {
                            Ok(None)
                        }
                    })
                    .expect("Failed to spawn subtrie thread");
                Some((nibble, handle))
            })
            .collect();

        // Collect subtrie roots and merge into root BranchNode
        let mut root_choices = BranchNode::EMPTY_CHOICES;
        for (nibble, handle) in handles {
            let branch_opt = handle
                .join()
                .map_err(|_| TrieGenerationError::ThreadJoinError())??;
            if let Some(branch) = branch_opt {
                root_choices[nibble] = branch.choices[nibble].clone();
            }
        }

        // Write root BranchNode to DB
        let root_node: Node = BranchNode {
            choices: root_choices,
            value: vec![],
        }
        .into();
        let mut buf = Vec::with_capacity(512);
        let hash = root_node.compute_hash_no_alloc(&mut buf).finalize();
        db.put_batch_no_alloc(&[(Nibbles::default(), root_node)])
            .map_err(TrieGenerationError::FlushToDbError)?;

        Ok(hash)
    })
}

/// Wrapper function for `trie_from_sorted_accounts` that handles concurrency
/// and memory limits
pub fn trie_from_sorted_accounts_wrap<T>(
    db: &dyn TrieDB,
    accounts_iter: &mut T,
) -> Result<H256, TrieGenerationError>
where
    T: Iterator<Item = (H256, Vec<u8>)> + Send,
{
    let (buffer_sender, buffer_receiver) = bounded::<Vec<(Nibbles, Node)>>(BUFFER_COUNT as usize);
    for _ in 0..BUFFER_COUNT {
        let _ = buffer_sender.send(Vec::with_capacity(SIZE_TO_WRITE_DB as usize));
    }
    scope(|s| {
        let pool = ThreadPool::new(12, s);
        trie_from_sorted_accounts(
            db,
            accounts_iter,
            Arc::new(pool),
            buffer_sender,
            buffer_receiver,
        )
    })
}

#[cfg(test)]
mod test {
    use ethereum_types::U256;
    use ethrex_rlp::encode::RLPEncode;

    use crate::{InMemoryTrieDB, Trie};

    use super::*;
    use std::{collections::BTreeMap, str::FromStr, sync::Mutex};

    fn generate_input_1() -> BTreeMap<H256, Vec<u8>> {
        let mut accounts: BTreeMap<H256, Vec<u8>> = BTreeMap::new();
        for string in [
            "68521f7430502aef983fd7568ea179ed0f8d12d5b68883c90573781ae0778ec2",
            "68db10f720d5972738df0d841d64c7117439a1a2ca9ba247e7239b19eb187414",
            "6b7c1458952b903dbe3717bc7579f18e5cb1136be1b11b113cdac0f0791c07d3",
        ] {
            accounts.insert(H256::from_str(string).unwrap(), vec![0, 1]);
        }
        accounts
    }

    fn generate_input_2() -> BTreeMap<H256, Vec<u8>> {
        let mut accounts: BTreeMap<H256, Vec<u8>> = BTreeMap::new();
        for string in [
            "0532f23d3bd5277790ece5a6cb6fc684bc473a91ffe3a0334049527c4f6987e9",
            "14d5df819167b77851220ee266178aee165daada67ca865e9d50faed6b4fdbe3",
            "6908aa86b715fcf221f208a28bb84bf6359ba9c41da04b7e17a925cdb22bf704",
            "90bbe47533cd80b5d9cef6c283415edd90296bf4ac4ede6d2a6b42bb3d5e7d0e",
            "90c2fdad333366cf0f18f0dded9b478590c0563e4c847c79aee0b733b5a9104f",
            "af9e3efce873619102dfdb0504abd44179191bccfb624608961e71492a1ba5b7",
            "b723d5841dc4d6d3fe7de03ad74dd83798c3b68f752bba29c906ec7f5a469452",
            "c2c6fd64de59489f0c27e75443c24327cef6415f1d3ee1659646abefab212113",
            "ca0d791e7a3e0f25d775034acecbaaf9219939288e6282d8291e181b9c3c24b0",
            "f0dcaaa40dfc67925d6e172e48b8f83954ba46cfb1bb522c809f3b93b49205ee",
        ] {
            accounts.insert(H256::from_str(string).unwrap(), vec![0, 1]);
        }
        accounts
    }

    fn generate_input_3() -> BTreeMap<H256, Vec<u8>> {
        let mut accounts: BTreeMap<H256, Vec<u8>> = BTreeMap::new();
        for string in [
            "0532f23d3bd5277790ece5a6cb6fc684bc473a91ffe3a0334049527c4f6987e9",
            "0542f23d3bd5277790ece5a6cb6fc684bc473a91ffe3a0334049527c4f6987e9",
            "0552f23d3bd5277790ece5a6cb6fc684bc473a91ffe3a0334049527c4f6987e9",
        ] {
            accounts.insert(H256::from_str(string).unwrap(), vec![0, 1]);
        }
        accounts
    }

    fn generate_input_4() -> BTreeMap<H256, Vec<u8>> {
        let mut accounts: BTreeMap<H256, Vec<u8>> = BTreeMap::new();
        let string = "0532f23d3bd5277790ece5a6cb6fc684bc473a91ffe3a0334049527c4f6987e9";
        accounts.insert(H256::from_str(string).unwrap(), vec![0, 1]);
        accounts
    }

    fn generate_input_5() -> BTreeMap<H256, Vec<u8>> {
        let mut accounts: BTreeMap<H256, Vec<u8>> = BTreeMap::new();
        for (string, value) in [
            (
                "290decd9548b62a8d60345a988386fc84ba6bc95484008f6362f93160ef3e563",
                U256::from_str("1191240792495687806002885977912460542139236513636").unwrap(),
            ),
            (
                "295841a49a1089f4b560f91cfbb0133326654dcbb1041861fc5dde96c724a22f",
                U256::from(480),
            ),
        ] {
            accounts.insert(H256::from_str(string).unwrap(), value.encode_to_vec());
        }
        accounts
    }

    fn generate_input_slots_1() -> BTreeMap<H256, U256> {
        let mut slots: BTreeMap<H256, U256> = BTreeMap::new();
        for string in [
            "0532f23d3bd5277790ece5a6cb6fc684bc473a91ffe3a0334049527c4f6987e8",
            "0532f23d3bd5277790ece5a6cb6fc684bc473a91ffe3a0334049527c4f6987e9",
            "0552f23d3bd5277790ece5a6cb6fc684bc473a91ffe3a0334049527c4f6987e9",
        ] {
            slots.insert(H256::from_str(string).unwrap(), U256::zero());
        }
        slots
    }

    pub fn run_test_account_state(accounts: BTreeMap<H256, Vec<u8>>) {
        let computed_data = Arc::new(Mutex::new(BTreeMap::new()));
        let trie = Trie::new(Box::new(InMemoryTrieDB::new(computed_data.clone())));
        let db = trie.db();
        let tested_trie_hash: H256 = trie_from_sorted_accounts_wrap(
            db,
            &mut accounts
                .clone()
                .into_iter()
                .map(|(hash, state)| (hash, state.encode_to_vec())),
        )
        .expect("Shouldn't have errors");

        let expected_data = Arc::new(Mutex::new(BTreeMap::new()));
        let mut trie = Trie::new(Box::new(InMemoryTrieDB::new(expected_data.clone())));
        for account in accounts.iter() {
            trie.insert(account.0.as_bytes().to_vec(), account.1.encode_to_vec())
                .unwrap();
        }

        assert_eq!(tested_trie_hash, trie.hash().unwrap());

        let computed_data = computed_data.lock().unwrap();
        let expected_data = expected_data.lock().unwrap();
        for (k, v) in expected_data.iter() {
            // skip flatkeyvalues, we don't want them
            if k.last().cloned() == Some(16) {
                continue;
            }
            assert!(computed_data.contains_key(k));
            assert_eq!(*v, computed_data[k]);
        }
    }

    pub fn run_test_storage_slots(slots: BTreeMap<H256, U256>) {
        let trie = Trie::stateless();
        let db = trie.db();
        let tested_trie_hash: H256 = trie_from_sorted_accounts_wrap(
            db,
            &mut slots
                .clone()
                .into_iter()
                .map(|(hash, state)| (hash, state.encode_to_vec())),
        )
        .expect("Shouldn't have errors");

        let mut trie: Trie = Trie::empty_in_memory();
        for account in slots.iter() {
            trie.insert(account.0.as_bytes().to_vec(), account.1.encode_to_vec())
                .unwrap();
        }

        let trie_hash = trie.hash_no_commit();

        assert!(tested_trie_hash == trie_hash)
    }

    /// Verifies that the parallel trie builder produces the same root hash and
    /// DB contents as the sequential builder
    fn run_test_parallel(accounts: BTreeMap<H256, Vec<u8>>) {
        // Build with sequential algorithm
        let seq_data = Arc::new(Mutex::new(BTreeMap::new()));
        let seq_trie = Trie::new(Box::new(InMemoryTrieDB::new(seq_data.clone())));
        let seq_hash: H256 = trie_from_sorted_accounts_wrap(
            seq_trie.db(),
            &mut accounts
                .clone()
                .into_iter()
                .map(|(hash, state)| (hash, state.encode_to_vec())),
        )
        .expect("Sequential build failed");

        // Build with parallel algorithm
        let par_data = Arc::new(Mutex::new(BTreeMap::new()));
        let par_trie = Trie::new(Box::new(InMemoryTrieDB::new(par_data.clone())));
        let par_hash: H256 = trie_from_sorted_accounts_parallel(
            par_trie.db(),
            &mut accounts
                .into_iter()
                .map(|(hash, state)| (hash, state.encode_to_vec())),
        )
        .expect("Parallel build failed");

        assert_eq!(seq_hash, par_hash, "Root hashes differ");

        let seq_data = seq_data.lock().unwrap();
        let par_data = par_data.lock().unwrap();
        for (k, v) in seq_data.iter() {
            assert!(
                par_data.contains_key(k),
                "Parallel DB missing key: {k:?}"
            );
            assert_eq!(
                *v, par_data[k],
                "Value mismatch for key: {k:?}"
            );
        }
    }

    #[test]
    fn test_1() {
        run_test_account_state(generate_input_1());
    }

    #[test]
    fn test_2() {
        run_test_account_state(generate_input_2());
    }

    #[test]
    fn test_3() {
        run_test_account_state(generate_input_3());
    }

    #[test]
    fn test_4() {
        run_test_account_state(generate_input_4());
    }

    #[test]
    fn test_5() {
        run_test_account_state(generate_input_5());
    }

    #[test]
    fn test_slots_1() {
        run_test_storage_slots(generate_input_slots_1());
    }

    #[test]
    fn test_parallel_multi_nibble() {
        // generate_input_2 has entries spanning multiple first nibbles (0, 1, 6, 9, a, b, c, f)
        run_test_parallel(generate_input_2());
    }

    #[test]
    fn test_parallel_single_nibble() {
        // generate_input_1 has all entries starting with nibble 6 — falls back to sequential
        run_test_parallel(generate_input_1());
    }

    #[test]
    fn test_parallel_same_prefix() {
        // generate_input_3 has all entries starting with 05 — falls back to sequential
        run_test_parallel(generate_input_3());
    }

    #[test]
    fn test_parallel_single_item() {
        run_test_parallel(generate_input_4());
    }

    #[test]
    fn test_parallel_empty() {
        let accounts: BTreeMap<H256, Vec<u8>> = BTreeMap::new();
        let par_data = Arc::new(Mutex::new(BTreeMap::new()));
        let par_trie = Trie::new(Box::new(InMemoryTrieDB::new(par_data)));
        let hash = trie_from_sorted_accounts_parallel(
            par_trie.db(),
            &mut accounts.into_iter(),
        )
        .expect("Parallel build failed");
        assert_eq!(hash, *EMPTY_TRIE_HASH);
    }
}
