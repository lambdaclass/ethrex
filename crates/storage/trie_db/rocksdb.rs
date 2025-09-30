use ethrex_common::H256;
use ethrex_trie::{Nibbles, NodeHash, TrieDB, error::TrieError};
use rocksdb::{DBWithThreadMode, MultiThreaded, OptimisticTransactionDB};
use std::sync::Arc;

use crate::trie_db::layering::apply_prefix;

/// RocksDB implementation for the TrieDB trait, with get and put operations.
pub struct RocksDBTrieDB {
    /// RocksDB database
    db: Arc<OptimisticTransactionDB<MultiThreaded>>,
    /// Column family name
    cf_name: String,
    /// Storage trie address prefix
    address_prefix: Option<H256>,
}

impl RocksDBTrieDB {
    pub fn new(
        db: Arc<OptimisticTransactionDB<MultiThreaded>>,
        cf_name: &str,
        address_prefix: Option<H256>,
    ) -> Result<Self, TrieError> {
        // Verify column family exists
        if db.cf_handle(cf_name).is_none() {
            return Err(TrieError::DbError(anyhow::anyhow!(
                "Column family not found: {}",
                cf_name
            )));
        }

        Ok(Self {
            db,
            cf_name: cf_name.to_string(),
            address_prefix,
        })
    }

    fn cf_handle(&self) -> Result<std::sync::Arc<rocksdb::BoundColumnFamily>, TrieError> {
        self.db
            .cf_handle(&self.cf_name)
            .ok_or_else(|| TrieError::DbError(anyhow::anyhow!("Column family not found")))
    }

    fn make_key(&self, node_hash: Nibbles) -> Vec<u8> {
        apply_prefix(self.address_prefix, node_hash)
            .as_ref()
            .to_vec()
    }
}

impl TrieDB for RocksDBTrieDB {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let cf = self.cf_handle()?;
        let db_key = self.make_key(key);

        let res = self
            .db
            .get_cf(&cf, &db_key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB get error: {}", e)))?;

        Ok(res)
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let cf = self.cf_handle()?;
        let mut batch = rocksdb::WriteBatchWithTransaction::default();

        for (key, value) in key_values {
            let db_key = self.make_key(key);
            if value.is_empty() {
                batch.delete_cf(&cf, db_key);
            } else {
                batch.put_cf(&cf, db_key, value);
            }
        }

        self.db
            .write(batch)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB batch write error: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_trie::{Nibbles, NodeHash};
    use rocksdb::{ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded, Options};
    use tempfile::TempDir;

    #[test]
    fn test_trie_db_basic_operations() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_db");

        // Setup RocksDB with column family
        let mut db_options = Options::default();
        db_options.create_if_missing(true);
        db_options.create_missing_column_families(true);

        let cf_descriptor = ColumnFamilyDescriptor::new("test_cf", Options::default());
        let db = OptimisticTransactionDB::<MultiThreaded>::open_cf_descriptors(
            &db_options,
            db_path,
            vec![cf_descriptor],
        )
        .unwrap();
        let db = Arc::new(db);

        // Create TrieDB
        let trie_db = RocksDBTrieDB::new(db, "test_cf", None).unwrap();

        // Test data
        let node_hash = Nibbles::from_hex(vec![1]);
        let node_data = vec![1, 2, 3, 4, 5];

        // Test put_batch
        trie_db
            .put_batch(vec![(node_hash.clone(), node_data.clone())])
            .unwrap();

        // Test get
        let retrieved_data = trie_db.get(node_hash).unwrap().unwrap();
        assert_eq!(retrieved_data, node_data);

        // Test get nonexistent
        let nonexistent_hash = Nibbles::from_hex(vec![2]);
        assert!(trie_db.get(nonexistent_hash).unwrap().is_none());
    }

    #[test]
    fn test_trie_db_with_address_prefix() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_db");

        // Setup RocksDB with column family
        let mut db_options = Options::default();
        db_options.create_if_missing(true);
        db_options.create_missing_column_families(true);

        let cf_descriptor = ColumnFamilyDescriptor::new("test_cf", Options::default());
        let db = OptimisticTransactionDB::<MultiThreaded>::open_cf_descriptors(
            &db_options,
            db_path,
            vec![cf_descriptor],
        )
        .unwrap();
        let db = Arc::new(db);

        // Create TrieDB with address prefix
        let address = H256::from([0xaa; 32]);
        let trie_db = RocksDBTrieDB::new(db, "test_cf", Some(address)).unwrap();

        // Test data
        let node_hash = Nibbles::from_hex(vec![1]);
        let node_data = vec![1, 2, 3, 4, 5];

        // Test put_batch
        trie_db
            .put_batch(vec![(node_hash.clone(), node_data.clone())])
            .unwrap();

        // Test get
        let retrieved_data = trie_db.get(node_hash).unwrap().unwrap();
        assert_eq!(retrieved_data, node_data);
    }

    #[test]
    fn test_trie_db_batch_operations() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_db");

        // Setup RocksDB with column family
        let mut db_options = Options::default();
        db_options.create_if_missing(true);
        db_options.create_missing_column_families(true);

        let cf_descriptor = ColumnFamilyDescriptor::new("test_cf", Options::default());
        let db = OptimisticTransactionDB::<MultiThreaded>::open_cf_descriptors(
            &db_options,
            db_path,
            vec![cf_descriptor],
        )
        .unwrap();
        let db = Arc::new(db);

        // Create TrieDB
        let trie_db = RocksDBTrieDB::new(db, "test_cf", None).unwrap();

        // Test data
        let batch_data = vec![
            (Nibbles::from_hex(vec![1]), vec![1, 2, 3]),
            (Nibbles::from_hex(vec![1]), vec![4, 5, 6]),
            (Nibbles::from_hex(vec![1]), vec![7, 8, 9]),
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
