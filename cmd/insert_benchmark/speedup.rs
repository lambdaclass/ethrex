use std::{collections::BTreeMap, time::Instant};

use ethrex_common::{H256, U256, constants::EMPTY_KECCACK_HASH, types::AccountState};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store, error::StoreError};
use ethrex_trie::{
    EMPTY_TRIE_HASH, Nibbles, Node, Trie, TrieError,
    node::{BranchNode, ExtensionNode, LeafNode},
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use tracing::debug;

#[derive(Debug, Default, Clone)]
struct StackElement {
    path: Nibbles,
    element: BranchNode,
}

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
    StoreError(StoreError),
    #[error(transparent)]
    FlushToDbError(TrieError),
}

const SIZE_TO_WRITE_DB: u64 = 20_000;

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
    nodes_to_write: &mut Vec<Node>,
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
                nodes_to_write.push(node.clone().into());
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
    nodes_to_write.push(node);
    Ok(())
}

fn flush_nodes_to_write(
    nodes_to_write: Vec<Node>,
    store: Store,
    account: Option<H256>,
) -> Result<(), TrieGenerationError> {
    let trie = match account {
        Some(account) => store
            .open_locked_storage_trie(account, *EMPTY_TRIE_HASH)
            .map_err(TrieGenerationError::StoreError)?,
        None => store
            .open_state_trie(*EMPTY_TRIE_HASH)
            .map_err(TrieGenerationError::StoreError)?,
    };
    let db = trie.db();
    db.put_batch(
        nodes_to_write
            .par_iter()
            .map(|node| (node.compute_hash(), node.encode_to_vec()))
            .collect(),
    )
    .map_err(TrieGenerationError::FlushToDbError)
}

#[inline(never)]
pub async fn trie_from_sorted_accounts<'a, T>(
    store: Store,
    accounts_iter: &mut T,
    account_hash: Option<H256>,
) -> Result<Trie, TrieGenerationError>
where
    T: Iterator<Item = (H256, Vec<u8>)>,
{
    let mut nodes_to_write: Vec<Node> = Vec::with_capacity(20_065);
    let mut trie_stack: Vec<StackElement> = Vec::new();
    let mut db_joinset = tokio::task::JoinSet::new();

    let mut left_side = StackElement::default();
    let mut center_side: CenterSide = CenterSide::from_value(accounts_iter.next().unwrap());
    let mut right_side_opt: Option<(H256, Vec<u8>)> = accounts_iter.next();

    while let Some(right_side) = right_side_opt {
        if nodes_to_write.len() as u64 > SIZE_TO_WRITE_DB {
            if !db_joinset.is_empty() {
                db_joinset.join_next().await;
            }
            let store_clone = store.clone();
            db_joinset.spawn_blocking(move || {
                flush_nodes_to_write(nodes_to_write, store_clone, account_hash)
            });
            nodes_to_write = Vec::new();
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
        right_side_opt = accounts_iter.next();
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

        debug_assert!(nodes_to_write.last().unwrap().compute_hash() == child.compute_hash());
        match nodes_to_write.iter_mut().last().unwrap() {
            Node::Branch(_) => {
                nodes_to_write.push(
                    ExtensionNode {
                        prefix: Nibbles::from_hex(vec![index as u8]),
                        child,
                    }
                    .into(),
                );
                nodes_to_write
                    .last()
                    .expect("we just inserted")
                    .compute_hash()
                    .finalize()
            }
            Node::Extension(extension_node) => {
                extension_node.prefix.data.insert(0, index as u8);
                extension_node.compute_hash().finalize()
            }
            Node::Leaf(leaf_node) => leaf_node.compute_hash().finalize(),
        }
    } else {
        nodes_to_write.push(left_side.element.into());
        nodes_to_write
            .last()
            .expect("we just inserted")
            .compute_hash()
            .finalize()
    };

    if !db_joinset.is_empty() {
        db_joinset.join_next().await;
    }
    flush_nodes_to_write(nodes_to_write, store.clone(), account_hash)?;

    store
        .open_state_trie(hash)
        .map_err(TrieGenerationError::StoreError)
}

const TEST_SIZE: usize = 100_000;

#[tokio::main]
async fn main() {
    for _ in 0..1 {
        let store_engine = EngineType::RocksDB;
        let store = Store::new("test", store_engine).unwrap();
        let mut accounts: BTreeMap<H256, AccountState> = BTreeMap::new();
        for _ in 0..TEST_SIZE {
            accounts.insert(
                H256::random(),
                AccountState {
                    nonce: 0,
                    balance: U256::zero(),
                    storage_root: *EMPTY_TRIE_HASH,
                    code_hash: *EMPTY_KECCACK_HASH,
                },
            );
        }
        //let accounts = generate_input_3();

        let mut trie: Trie = store.open_state_trie(*EMPTY_TRIE_HASH).unwrap();
        let now: Instant = Instant::now();
        for account in accounts.iter() {
            trie.insert(account.0.as_bytes().to_vec(), account.1.encode_to_vec())
                .unwrap();
        }
        let state_root = trie.hash().unwrap();
        println!("Time in old fashioned {:?}", now.elapsed());

        let store = Store::new("test_fast", store_engine).unwrap();
        let now: Instant = Instant::now();
        let res: Trie = trie_from_sorted_accounts(
            store,
            &mut accounts
                .into_iter()
                .map(|(hash, state)| (hash, state.encode_to_vec())),
            None,
        )
        .await
        .expect("Shouldn't have errors");
        let computed_state_root = res.hash_no_commit();
        println!("Time in new fashioned {:?}", now.elapsed());
        let result = computed_state_root == state_root;
        println!("{result}");
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::str::FromStr;

    fn generate_input_1() -> BTreeMap<H256, AccountState> {
        let mut accounts: BTreeMap<H256, AccountState> = BTreeMap::new();
        for string in [
            "68521f7430502aef983fd7568ea179ed0f8d12d5b68883c90573781ae0778ec2",
            "68db10f720d5972738df0d841d64c7117439a1a2ca9ba247e7239b19eb187414",
            "6b7c1458952b903dbe3717bc7579f18e5cb1136be1b11b113cdac0f0791c07d3",
        ] {
            accounts.insert(
                H256::from_str(string).unwrap(),
                AccountState {
                    nonce: 0,
                    balance: U256::zero(),
                    storage_root: *EMPTY_TRIE_HASH,
                    code_hash: *EMPTY_KECCACK_HASH,
                },
            );
        }
        accounts
    }

    fn generate_input_2() -> BTreeMap<H256, AccountState> {
        let mut accounts: BTreeMap<H256, AccountState> = BTreeMap::new();
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
            accounts.insert(
                H256::from_str(string).unwrap(),
                AccountState {
                    nonce: 0,
                    balance: U256::zero(),
                    storage_root: *EMPTY_TRIE_HASH,
                    code_hash: *EMPTY_KECCACK_HASH,
                },
            );
        }
        accounts
    }

    fn generate_input_3() -> BTreeMap<H256, AccountState> {
        let mut accounts: BTreeMap<H256, AccountState> = BTreeMap::new();
        for string in [
            "0532f23d3bd5277790ece5a6cb6fc684bc473a91ffe3a0334049527c4f6987e9",
            "0542f23d3bd5277790ece5a6cb6fc684bc473a91ffe3a0334049527c4f6987e9",
            "0552f23d3bd5277790ece5a6cb6fc684bc473a91ffe3a0334049527c4f6987e9",
        ] {
            accounts.insert(
                H256::from_str(string).unwrap(),
                AccountState {
                    nonce: 0,
                    balance: U256::zero(),
                    storage_root: *EMPTY_TRIE_HASH,
                    code_hash: *EMPTY_KECCACK_HASH,
                },
            );
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

    pub async fn run_test_account_state(accounts: BTreeMap<H256, AccountState>) {
        let store =
            Store::new("memory", EngineType::InMemory).expect("Should open the inmemory db");
        let tested_trie: Trie = trie_from_sorted_accounts(
            store,
            &mut accounts
                .clone()
                .into_iter()
                .map(|(hash, state)| (hash, state.encode_to_vec())),
            None,
        )
        .await
        .expect("Shouldn't have errors");

        let mut trie: Trie = Trie::empty_in_memory();
        for account in accounts.iter() {
            trie.insert(account.0.as_bytes().to_vec(), account.1.encode_to_vec())
                .unwrap();
        }

        assert!(tested_trie.hash_no_commit() == trie.hash_no_commit())
    }

    pub async fn run_test_storage_slots(slots: BTreeMap<H256, U256>) {
        let account_hash = Some(H256::zero());
        let store =
            Store::new("memory", EngineType::InMemory).expect("Should open the inmemory db");
        let tested_trie: Trie = trie_from_sorted_accounts(
            store,
            &mut slots
                .clone()
                .into_iter()
                .map(|(hash, state)| (hash, state.encode_to_vec())),
            account_hash,
        )
        .await
        .expect("Shouldn't have errors");

        let mut trie: Trie = Trie::empty_in_memory();
        for account in slots.iter() {
            trie.insert(account.0.as_bytes().to_vec(), account.1.encode_to_vec())
                .unwrap();
        }

        let trie_hash = trie.hash_no_commit();
        let tested_trie_hash = tested_trie.hash_no_commit();

        assert!(tested_trie_hash == trie_hash)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_1() {
        run_test_account_state(generate_input_1()).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_2() {
        run_test_account_state(generate_input_2()).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_3() {
        run_test_account_state(generate_input_3()).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_slots_1() {
        run_test_storage_slots(generate_input_slots_1()).await;
    }
}
