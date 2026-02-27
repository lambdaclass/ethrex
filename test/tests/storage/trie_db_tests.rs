use ethrex_common::H256;
use ethrex_storage::backend::in_memory::InMemoryBackend;
use ethrex_storage::trie::BackendTrieDB;
use ethrex_trie::{Nibbles, TrieDB};
use std::sync::Arc;

#[test]
fn test_trie_db_basic_operations() {
    let backend = Arc::new(InMemoryBackend::open().unwrap());

    // Create TrieDB and write data
    let trie_db = BackendTrieDB::new_for_accounts(backend.clone(), vec![]).unwrap();

    let node_hash = Nibbles::from_hex(vec![1]);
    let node_data = vec![1, 2, 3, 4, 5];

    trie_db
        .put_batch(vec![(node_hash.clone(), node_data.clone())])
        .unwrap();

    // Create a fresh TrieDB to read back (read view is a snapshot from construction time)
    let trie_db = BackendTrieDB::new_for_accounts(backend, vec![]).unwrap();

    let retrieved_data = trie_db.get(node_hash).unwrap().unwrap();
    assert_eq!(retrieved_data, node_data);

    // Test get nonexistent
    let nonexistent_hash = Nibbles::from_hex(vec![2]);
    assert!(trie_db.get(nonexistent_hash).unwrap().is_none());
}

#[test]
fn test_trie_db_with_address_prefix() {
    let backend = Arc::new(InMemoryBackend::open().unwrap());

    // Create TrieDB with address prefix and write data
    let address = H256::from([0xaa; 32]);
    let trie_db = BackendTrieDB::new_for_account_storage(backend.clone(), address, vec![]).unwrap();

    let node_hash = Nibbles::from_hex(vec![1]);
    let node_data = vec![1, 2, 3, 4, 5];

    trie_db
        .put_batch(vec![(node_hash.clone(), node_data.clone())])
        .unwrap();

    // Create a fresh TrieDB to read back
    let trie_db = BackendTrieDB::new_for_account_storage(backend, address, vec![]).unwrap();

    let retrieved_data = trie_db.get(node_hash).unwrap().unwrap();
    assert_eq!(retrieved_data, node_data);
}

#[test]
fn test_trie_db_batch_operations() {
    let backend = Arc::new(InMemoryBackend::open().unwrap());

    // Create TrieDB and write batch data
    let trie_db = BackendTrieDB::new_for_accounts(backend.clone(), vec![]).unwrap();

    // NOTE: we don't use the same paths to avoid overwriting in the batch
    let batch_data = vec![
        (Nibbles::from_hex(vec![1]), vec![1, 2, 3]),
        (Nibbles::from_hex(vec![1, 2]), vec![4, 5, 6]),
        (Nibbles::from_hex(vec![1, 2, 3]), vec![7, 8, 9]),
    ];

    trie_db.put_batch(batch_data.clone()).unwrap();

    // Create a fresh TrieDB to read back
    let trie_db = BackendTrieDB::new_for_accounts(backend, vec![]).unwrap();

    for (node_hash, expected_data) in batch_data {
        let retrieved_data = trie_db.get(node_hash).unwrap().unwrap();
        assert_eq!(retrieved_data, expected_data);
    }
}
