use ethrex_common::H256;
use ethrex_trie::{NodeHash, TrieDB, error::TrieError};
use rocksdb::{BoundColumnFamily, DBWithThreadMode, MultiThreaded};
use std::sync::Arc;

/// RocksDB implementation for the TrieDB trait, with get and put operations.
pub struct RocksDBTrieDB {
    /// RocksDB database
    primary: &'static Arc<DBWithThreadMode<MultiThreaded>>,
    secondary: &'static Arc<DBWithThreadMode<MultiThreaded>>,
    /// Column handles
    cf_primary: Arc<BoundColumnFamily<'static>>,
    cf_secondary: Arc<BoundColumnFamily<'static>>,
    /// Storage trie address prefix
    address_prefix: Option<H256>,
}

impl RocksDBTrieDB {
    pub fn new(
        primary: Arc<DBWithThreadMode<MultiThreaded>>,
        secondary: Arc<DBWithThreadMode<MultiThreaded>>,
        cf_name: &str,
        address_prefix: Option<H256>,
    ) -> Result<Self, TrieError> {
        // Verify column family exists
        let primary = Box::leak(Box::new(primary));
        let secondary = Box::leak(Box::new(secondary));
        let cf_primary = primary.cf_handle(cf_name).ok_or_else(|| {
            TrieError::DbError(anyhow::anyhow!("Column family not found: {}", cf_name))
        })?;
        let cf_secondary = secondary.cf_handle(cf_name).ok_or_else(|| {
            TrieError::DbError(anyhow::anyhow!("Column family not found: {}", cf_name))
        })?;

        Ok(Self {
            primary,
            secondary,
            cf_primary,
            cf_secondary,
            address_prefix,
        })
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
                // For state trie, use node hash directly
                node_hash.as_ref().to_vec()
            }
        }
    }
}

impl Drop for RocksDBTrieDB {
    fn drop(&mut self) {
        // Restore the leaked database reference
        for db in [self.secondary, self.primary] {
            unsafe {
                drop(Box::from_raw(
                    db as *const Arc<DBWithThreadMode<MultiThreaded>>
                        as *mut Arc<DBWithThreadMode<MultiThreaded>>,
                ));
            }
        }
    }
}

impl TrieDB for RocksDBTrieDB {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let db_key = self.make_key(key);

        self.secondary
            .get_cf(&self.cf_secondary, db_key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB get error: {}", e)))
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let mut batch = rocksdb::WriteBatch::default();

        for (key, value) in key_values {
            let db_key = self.make_key(key);
            batch.put_cf(&self.cf_primary, db_key, value);
        }

        self.primary
            .write(batch)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB batch write error: {}", e)))?;
        self.secondary.try_catch_up_with_primary().map_err(|e| {
            TrieError::DbError(anyhow::anyhow!(
                "Secondary RocksDB instance failed to catch up with primary: {}",
                e
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_trie::NodeHash;
    use rocksdb::{ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded, Options};
    use tempfile::TempDir;

    #[track_caller]
    fn open_dbs() -> [Arc<DBWithThreadMode<MultiThreaded>>; 2] {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_db");

        // Setup RocksDB with column family
        let mut db_options = Options::default();
        db_options.create_if_missing(true);
        db_options.create_missing_column_families(true);

        let cf_descriptor = ColumnFamilyDescriptor::new("test_cf", Options::default());
        let primary = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
            &db_options,
            db_path.clone(),
            vec![cf_descriptor],
        )
        .unwrap();
        let secondary = DBWithThreadMode::<MultiThreaded>::open_as_secondary(
            &db_options,
            db_path.clone(),
            db_path.join(".secondary"),
        )
        .unwrap();
        [Arc::new(primary), Arc::new(secondary)]
    }

    #[test]
    fn test_trie_db_basic_operations() {
        let [primary, secondary] = open_dbs();
        // Create TrieDB
        let trie_db = RocksDBTrieDB::new(primary, secondary, "test_cf", None).unwrap();

        // Test data
        let node_hash = NodeHash::from(H256::from([1u8; 32]));
        let node_data = vec![1, 2, 3, 4, 5];

        // Test put_batch
        trie_db
            .put_batch(vec![(node_hash, node_data.clone())])
            .unwrap();

        // Test get
        let retrieved_data = trie_db.get(node_hash).unwrap().unwrap();
        assert_eq!(retrieved_data, node_data);

        // Test get nonexistent
        let nonexistent_hash = NodeHash::from(H256::from([2u8; 32]));
        assert!(trie_db.get(nonexistent_hash).unwrap().is_none());
    }

    #[test]
    fn test_trie_db_with_address_prefix() {
        let [primary, secondary] = open_dbs();

        // Create TrieDB with address prefix
        let address = H256::from([0xaa; 32]);
        let trie_db = RocksDBTrieDB::new(primary, secondary, "test_cf", Some(address)).unwrap();

        // Test data
        let node_hash = NodeHash::from(H256::from([1u8; 32]));
        let node_data = vec![1, 2, 3, 4, 5];

        // Test put_batch
        trie_db
            .put_batch(vec![(node_hash, node_data.clone())])
            .unwrap();

        // Test get
        let retrieved_data = trie_db.get(node_hash).unwrap().unwrap();
        assert_eq!(retrieved_data, node_data);
    }

    #[test]
    fn test_trie_db_batch_operations() {
        let [primary, secondary] = open_dbs();

        // Create TrieDB
        let trie_db = RocksDBTrieDB::new(primary, secondary, "test_cf", None).unwrap();

        // Test data
        let batch_data = vec![
            (NodeHash::from(H256::from([1u8; 32])), vec![1, 2, 3]),
            (NodeHash::from(H256::from([2u8; 32])), vec![4, 5, 6]),
            (NodeHash::from(H256::from([3u8; 32])), vec![7, 8, 9]),
        ];

        // Test batch put
        trie_db.put_batch(batch_data.clone()).unwrap();

        // Test batch get
        for (node_hash, expected_data) in batch_data {
            let retrieved_data = trie_db.get(node_hash).unwrap().unwrap();
            assert_eq!(retrieved_data, expected_data);
        }
    }
}
