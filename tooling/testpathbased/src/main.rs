use std::time::Instant;

use ethrex_common::H256;
use ethrex_storage::{EngineType, Store};
use ethrex_trie::EMPTY_TRIE_HASH;

const PREINSERT_SIZE: usize = 100_000_000;
const INSERT_SIZE: usize = 10_000;

#[tokio::main]
async fn main() {
    let store = Store::new("./store", EngineType::Libmdbx).expect("Failed to create Store");

    let start = Instant::now();
    let mut trie = store.open_state_trie(*EMPTY_TRIE_HASH).unwrap();
    for _ in 0..PREINSERT_SIZE {
        trie.insert(H256::random().0.to_vec(), vec![1u8; 100])
            .unwrap();
    }
    let root = trie.hash().unwrap();
    println!(
        "Preinsert ({PREINSERT_SIZE}) took {}ms",
        start.elapsed().as_millis()
    );

    let start = Instant::now();
    let mut trie = store.open_state_trie(root).unwrap();
    let mut paths = Vec::new();
    for _ in 0..INSERT_SIZE {
        let path = H256::random().0.to_vec();
        paths.push(path.clone());
        trie.insert(path, vec![1u8; 100]).unwrap();
    }
    let root = trie.hash().unwrap();
    println!(
        "Insert ({INSERT_SIZE}) took {}ms",
        start.elapsed().as_millis()
    );

    let start = Instant::now();
    let trie = store.open_state_trie(root).unwrap();
    for path in paths {
        assert_eq!(100, trie.get(&path).unwrap().unwrap().len());
    }
    println!(
        "Read ({INSERT_SIZE}) took {}ms",
        start.elapsed().as_millis()
    );
}
