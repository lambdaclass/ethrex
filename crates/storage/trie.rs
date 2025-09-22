use crate::api::{StorageBackend, StorageLocked};
use ethrex_common::H256;
use ethrex_trie::{NodeHash, TrieDB, error::TrieError};
use std::sync::Arc;

/// StorageBackend implementation for the TrieDB trait
/// Works with any database that implements StorageBackend
pub struct BackendTrieDB {
    /// Storage backend
    backend: Arc<dyn StorageBackend>,
    /// Table name for storing trie nodes
    table_name: String,
    /// Storage trie address prefix (for storage tries)
    /// None for state tries, Some(address) for storage tries
    address_prefix: Option<H256>,
}

impl BackendTrieDB {
    pub fn new(
        backend: Arc<dyn StorageBackend>,
        table_name: &str,
        address_prefix: Option<H256>,
    ) -> Self {
        Self {
            backend,
            table_name: table_name.to_string(),
            address_prefix,
        }
    }

    fn make_key(&self, node_hash: NodeHash) -> Vec<u8> {
        match &self.address_prefix {
            Some(address) => {
                // For storage tries, prefix with address
                let mut key = address.as_bytes().to_vec();
                key.extend_from_slice(node_hash.as_ref());
                key
            }
            None => {
                // For state tries, use node hash directly
                node_hash.as_ref().to_vec()
            }
        }
    }
}

impl TrieDB for BackendTrieDB {
    fn get(&self, node_hash: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let key = self.make_key(node_hash);
        let txn = self.backend.begin_read().map_err(|e| {
            TrieError::DbError(anyhow::anyhow!("Failed to begin read transaction: {}", e))
        })?;

        txn.get(&self.table_name, &key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put(&self, node_hash: NodeHash, value: Vec<u8>) -> Result<(), TrieError> {
        self.put_batch(vec![(node_hash, value)])
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let batch: Vec<(Vec<u8>, Vec<u8>)> = key_values
            .into_iter()
            .map(|(node_hash, value)| (self.make_key(node_hash), value))
            .collect();

        self.backend
            .write_batch(&self.table_name, batch)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to write batch: {}", e)))
    }
}

/// Read-only version with persistent locked transaction/snapshot for batch reads
pub struct BackendTrieDBLocked {
    lock: Box<dyn StorageLocked>,
    address_prefix: Option<H256>,
}

impl BackendTrieDBLocked {
    pub fn new(lock: Box<dyn StorageLocked>, address_prefix: Option<H256>) -> Self {
        Self {
            lock,
            address_prefix,
        }
    }

    fn make_key(&self, node_hash: NodeHash) -> Vec<u8> {
        match &self.address_prefix {
            Some(address) => {
                // Para storage tries, prefijar con la dirección
                let mut key = address.as_bytes().to_vec();
                key.extend_from_slice(node_hash.as_ref());
                key
            }
            None => {
                // Para state trie, usar el hash del nodo directamente
                node_hash.as_ref().to_vec()
            }
        }
    }
}

impl TrieDB for BackendTrieDBLocked {
    fn get(&self, node_hash: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let key = self.make_key(node_hash);
        self.lock
            .get(&key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put(&self, _node_hash: NodeHash, _value: Vec<u8>) -> Result<(), TrieError> {
        // Read-only locked storage, should not be used for puts
        Err(TrieError::DbError(anyhow::anyhow!(
            "Cannot put in read-only locked storage"
        )))
    }

    fn put_batch(&self, _key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        // Read-only locked storage, should not be used for puts
        Err(TrieError::DbError(anyhow::anyhow!(
            "Cannot put_batch in read-only locked storage"
        )))
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::v2::backend::rocksdb::RocksDBBackend;
//     use ethrex_trie::NodeHash;
//     use tempfile::TempDir;

//     #[test]
//     fn test_backend_trie_basic_operations() {
//         let temp_dir = TempDir::new().unwrap();
//         let backend = RocksDBBackend::open(temp_dir.path().to_str().unwrap()).unwrap();

//         // Create state trie (no address prefix)
//         let trie_db = BackendTrieDB::new(backend, "state_trie_nodes", None);

//         let node_hash = NodeHash::from_encoded_raw(b"test_node_hash_1234567890123456");
//         let value = b"test_value".to_vec();

//         // Test put and get
//         trie_db.put(node_hash, value.clone()).unwrap();
//         let retrieved = trie_db.get(node_hash).unwrap();
//         assert_eq!(retrieved, Some(value));

//         // Test non-existent key
//         let non_existent = NodeHash::from_encoded_raw(b"non_existent_key_1234567890123456");
//         assert_eq!(trie_db.get(non_existent).unwrap(), None);
//     }

//     #[test]
//     fn test_storage_trie_with_prefix() {
//         let temp_dir = TempDir::new().unwrap();
//         let backend = RocksDBBackend::open(temp_dir.path().to_str().unwrap()).unwrap();

//         let address = H256::from_low_u64_be(12345);
//         let trie_db = BackendTrieDB::new(backend, "storage_trie_nodes", Some(address));

//         let node_hash = NodeHash::from_encoded_raw(b"test_storage_node_1234567890123456");
//         let value = b"storage_value".to_vec();

//         // Test put and get with address prefix
//         trie_db.put(node_hash, value.clone()).unwrap();
//         let retrieved = trie_db.get(node_hash).unwrap();
//         assert_eq!(retrieved, Some(value));
//     }

//     #[test]
//     fn test_batch_operations() {
//         let temp_dir = TempDir::new().unwrap();
//         let backend = RocksDBBackend::new(temp_dir.path().to_str().unwrap()).unwrap();

//         let trie_db = BackendTrieDB::new(backend, "state_trie_nodes", None);

//         let batch = vec![
//             (
//                 NodeHash::from_encoded_raw(b"key1_1234567890123456789012345678901"),
//                 b"value1".to_vec(),
//             ),
//             (
//                 NodeHash::from_encoded_raw(b"key2_1234567890123456789012345678901"),
//                 b"value2".to_vec(),
//             ),
//             (
//                 NodeHash::from_encoded_raw(b"key3_1234567890123456789012345678901"),
//                 b"value3".to_vec(),
//             ),
//         ];

//         // Test batch put
//         trie_db.put_batch(batch.clone()).unwrap();

//         // Verify all values were stored
//         for (key, expected_value) in batch {
//             let retrieved = trie_db.get(key).unwrap();
//             assert_eq!(retrieved, Some(expected_value));
//         }
//     }
// }
