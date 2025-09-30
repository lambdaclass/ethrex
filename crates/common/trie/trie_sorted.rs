use crate::{
    EMPTY_TRIE_HASH, Nibbles, Node, NodeHash, TrieDB, TrieError,
    node::{BranchNode, ExtensionNode, LeafNode},
};
use crossbeam::channel::{Receiver, Sender, bounded};
use ethereum_types::H256;
use ethrex_threadpool::ThreadPool;
use std::{sync::Arc, thread::scope};
use tracing::debug;

#[derive(Debug, Default, Clone)]
struct StackElement {
    path: Nibbles,
    element: BranchNode,
}

// The large size isn't a performance problem because we use a single instance of this
// struct
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
enum CenterSideElement {
    Branch { node: BranchNode },
    Leaf { value: Vec<u8> },
}

#[derive(Debug, Clone)]
struct CenterSide {
    path: Nibbles,
    element: CenterSideElement,
}

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

pub const SIZE_TO_WRITE_DB: u64 = 20_000;
pub const BUFFER_COUNT: u64 = 100;

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

fn is_child(this: &Nibbles, other: &StackElement) -> bool {
    this.count_prefix(&other.path) == other.path.len()
}

fn create_parent(center_side: &CenterSide, closest_nibbles: &Nibbles) -> StackElement {
    let new_parent_nibbles = center_side
        .path
        .slice(0, center_side.path.count_prefix(closest_nibbles));
    StackElement {
        path: new_parent_nibbles,
        element: BranchNode {
            choices: BranchNode::EMPTY_CHOICES,
            value: vec![],
        },
    }
}

fn add_center_to_parent_and_write_queue(
    nodes_to_write: &mut Vec<(NodeHash, Node)>,
    center_side: &CenterSide,
    parent_element: &mut StackElement,
) -> Result<(), TrieGenerationError> {
    debug!("{:x?}", center_side.path);
    debug!("{:x?}", parent_element.path);
    let mut path = center_side.path.clone();
    path.skip_prefix(&parent_element.path);
    let index = path
        .next()
        .ok_or(TrieGenerationError::IndexNotFound(center_side.path.clone()))?;
    let node: Node = match &center_side.element {
        CenterSideElement::Branch { node } => {
            if path.is_empty() {
                node.clone().into()
            } else {
                let hash = node.compute_hash();
                nodes_to_write.push((hash, node.clone().into()));
                ExtensionNode {
                    prefix: path,
                    child: hash.into(),
                }
                .into()
            }
        }
        CenterSideElement::Leaf { value } => LeafNode {
            partial: path,
            value: value.clone(),
        }
        .into(),
    };
    parent_element.element.choices[index as usize] = node.compute_hash().into();
    debug!(
        "branch {:x?}",
        parent_element
            .element
            .choices
            .iter()
            .enumerate()
            .filter_map(|(index, child)| child.is_valid().then_some(index))
            .collect::<Vec<_>>()
    );
    nodes_to_write.push((node.compute_hash(), node));
    Ok(())
}

fn flush_nodes_to_write(
    mut nodes_to_write: Vec<(NodeHash, Node)>,
    db: &dyn TrieDB,
    sender: Sender<Vec<(NodeHash, Node)>>,
) -> Result<(), TrieGenerationError> {
    db.put_batch_no_alloc(&nodes_to_write)
        .map_err(TrieGenerationError::FlushToDbError)?;
    nodes_to_write.clear();
    let _ = sender.send(nodes_to_write);
    Ok(())
}

#[inline(never)]
pub fn trie_from_sorted_accounts<'scope, T>(
    db: &'scope dyn TrieDB,
    data_iter: &mut T,
    scope: Arc<ThreadPool<'scope>>,
    buffer_sender: Sender<Vec<(NodeHash, Node)>>,
    buffer_receiver: Receiver<Vec<(NodeHash, Node)>>,
) -> Result<H256, TrieGenerationError>
where
    T: Iterator<Item = (H256, Vec<u8>)> + Send,
{
    let mut nodes_to_write: Vec<(NodeHash, Node)> = buffer_receiver
        .recv()
        .expect("This channel shouldn't close");
    let mut trie_stack: Vec<StackElement> = Vec::with_capacity(64); // Optimized for H256

    let mut left_side = StackElement::default();
    let Some(initial_value) = data_iter.next() else {
        return Ok(*EMPTY_TRIE_HASH);
    };
    let mut center_side: CenterSide = CenterSide::from_value(initial_value.clone());
    let mut right_side_opt: Option<(H256, Vec<u8>)> = data_iter.next();

    // Edge Case
    if right_side_opt.is_none() {
        let node = LeafNode {
            partial: center_side.path,
            value: initial_value.1,
        };
        let hash = node.compute_hash();
        flush_nodes_to_write(vec![(hash, node.into())], db, buffer_sender)?;
        return Ok(hash.finalize());
    }

    while let Some(right_side) = right_side_opt {
        if nodes_to_write.len() as u64 > SIZE_TO_WRITE_DB {
            let buffer_sender = buffer_sender.clone();
            scope.execute_priority(Box::new(move || {
                let _ = flush_nodes_to_write(nodes_to_write, db, buffer_sender);
            }));
            nodes_to_write = buffer_receiver
                .recv()
                .expect("This channel shouldn't close");
        }

        let right_side_path = Nibbles::from_bytes(right_side.0.as_bytes());
        while !is_child(&right_side_path, &left_side) {
            add_center_to_parent_and_write_queue(
                &mut nodes_to_write,
                &center_side,
                &mut left_side,
            )?;
            let temp = CenterSide::from_stack_element(left_side);
            left_side = trie_stack.pop().ok_or(TrieGenerationError::TrieStackEmpty(
                center_side.path.clone(),
            ))?;
            center_side = temp;
        }

        if center_side.path.count_prefix(&left_side.path)
            >= center_side.path.count_prefix(&right_side_path)
        {
            add_center_to_parent_and_write_queue(
                &mut nodes_to_write,
                &center_side,
                &mut left_side,
            )?;
        } else {
            let mut element = create_parent(&center_side, &right_side_path);
            add_center_to_parent_and_write_queue(&mut nodes_to_write, &center_side, &mut element)?;
            trie_stack.push(left_side);
            left_side = element;
        }
        center_side = CenterSide::from_value(right_side);
        right_side_opt = data_iter.next();
    }

    while !is_child(&center_side.path, &left_side) {
        let temp = CenterSide::from_stack_element(left_side);
        left_side = trie_stack.pop().ok_or(TrieGenerationError::TrieStackEmpty(
            center_side.path.clone(),
        ))?;
        add_center_to_parent_and_write_queue(&mut nodes_to_write, &temp, &mut left_side)?;
    }

    add_center_to_parent_and_write_queue(&mut nodes_to_write, &center_side, &mut left_side)?;

    while let Some(mut parent_node) = trie_stack.pop() {
        add_center_to_parent_and_write_queue(
            &mut nodes_to_write,
            &CenterSide::from_stack_element(left_side),
            &mut parent_node,
        )?;
        left_side = parent_node;
    }

    let hash = if left_side
        .element
        .choices
        .iter()
        .filter(|choice| choice.is_valid())
        .count()
        == 1
    {
        let (index, child) = left_side
            .element
            .choices
            .into_iter()
            .enumerate()
            .find(|(_, child)| child.is_valid())
            .unwrap();

        debug_assert!(nodes_to_write.last().unwrap().0 == child.compute_hash());
        let (_, node_hash_ref) = nodes_to_write.iter_mut().last().unwrap();
        match node_hash_ref {
            Node::Branch(_) => {
                let node: Node = ExtensionNode {
                    prefix: Nibbles::from_hex(vec![index as u8]),
                    child,
                }
                .into();
                nodes_to_write.push((node.compute_hash(), node));
                nodes_to_write
                    .last()
                    .expect("we just inserted")
                    .0
                    .finalize()
            }
            Node::Extension(extension_node) => {
                extension_node.prefix.data.insert(0, index as u8);
                extension_node.compute_hash().finalize()
            }
            Node::Leaf(leaf_node) => leaf_node.compute_hash().finalize(),
        }
    } else {
        let node: Node = left_side.element.into();
        nodes_to_write.push((node.compute_hash(), node));
        nodes_to_write
            .last()
            .expect("we just inserted")
            .0
            .finalize()
    };

    let _ = flush_nodes_to_write(nodes_to_write, db, buffer_sender);
    Ok(hash)
}

pub fn trie_from_sorted_accounts_wrap<T>(
    db: &dyn TrieDB,
    accounts_iter: &mut T,
) -> Result<H256, TrieGenerationError>
where
    T: Iterator<Item = (H256, Vec<u8>)> + Send,
{
    let (buffer_sender, buffer_receiver) = bounded::<Vec<(NodeHash, Node)>>(BUFFER_COUNT as usize);
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

    use crate::Trie;

    use super::*;
    use std::{collections::BTreeMap, str::FromStr};

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
        let trie = Trie::stateless();
        let db = trie.db();
        let tested_trie_hash: H256 = trie_from_sorted_accounts_wrap(
            db,
            &mut accounts
                .clone()
                .into_iter()
                .map(|(hash, state)| (hash, state.encode_to_vec())),
        )
        .expect("Shouldn't have errors");

        let mut trie: Trie = Trie::empty_in_memory();
        for account in accounts.iter() {
            trie.insert(account.0.as_bytes().to_vec(), account.1.encode_to_vec())
                .unwrap();
        }

        assert!(tested_trie_hash == trie.hash_no_commit())
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
    fn test_slots_1() {
        run_test_storage_slots(generate_input_slots_1());
    }
}
