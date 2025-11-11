use ethrex_common::H256;
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::{Nibbles, Node, TrieDB, error::TrieError};
use rocksdb::{DBWithThreadMode, MultiThreaded};
use std::sync::Arc;

use crate::trie_db::layering::apply_prefix;

/// RocksDB implementation for the TrieDB trait, with get and put operations.
pub struct RocksDBTrieDB {
    /// RocksDB database
    db: Arc<DBWithThreadMode<MultiThreaded>>,
    /// Column family name for the trie nodes
    trie_cf_name: String,
    /// Column family name for the flatkeyvalue nodes
    flatkeyvalue_cf_name: String,
    /// Storage trie address prefix
    address_prefix: Option<H256>,
    /// Last flatkeyvalue path already generated
    last_computed_flatkeyvalue: Nibbles,
}

impl RocksDBTrieDB {
    pub fn new(
        db: Arc<DBWithThreadMode<MultiThreaded>>,
        trie_cf_name: &str,
        flatkeyvalue_cf_name: &str,
        address_prefix: Option<H256>,
        last_written: Vec<u8>,
    ) -> Result<Self, TrieError> {
        // Verify column family exists
        if db.cf_handle(trie_cf_name).is_none() {
            return Err(TrieError::DbError(anyhow::anyhow!(
                "Column family for the trie not found: {}",
                trie_cf_name
            )));
        }

        if db.cf_handle(flatkeyvalue_cf_name).is_none() {
            return Err(TrieError::DbError(anyhow::anyhow!(
                "Column family for the flatkeyvalue not found: {}",
                flatkeyvalue_cf_name
            )));
        }
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);

        Ok(Self {
            db,
            trie_cf_name: trie_cf_name.to_string(),
            flatkeyvalue_cf_name: flatkeyvalue_cf_name.to_string(),
            address_prefix,
            last_computed_flatkeyvalue,
        })
    }

    fn cf_handle(&self) -> Result<std::sync::Arc<rocksdb::BoundColumnFamily<'_>>, TrieError> {
        self.db.cf_handle(&self.trie_cf_name).ok_or_else(|| {
            TrieError::DbError(anyhow::anyhow!("Column family for the trie not found"))
        })
    }

    fn cf_handle_flatkeyvalue(
        &self,
    ) -> Result<std::sync::Arc<rocksdb::BoundColumnFamily<'_>>, TrieError> {
        self.db
            .cf_handle(&self.flatkeyvalue_cf_name)
            .ok_or_else(|| {
                TrieError::DbError(anyhow::anyhow!(
                    "Column family for the flat key value store not found"
                ))
            })
    }

    fn make_key(&self, node_hash: Nibbles) -> Vec<u8> {
        apply_prefix(self.address_prefix, node_hash)
            .as_ref()
            .to_vec()
    }

    // Gets the correct column family handle based on whether the key is a leaf or not.
    fn cf_handle_for_key(
        &self,
        key: &Nibbles,
    ) -> Result<std::sync::Arc<rocksdb::BoundColumnFamily<'_>>, TrieError> {
        if key.is_leaf() {
            self.cf_handle_flatkeyvalue()
        } else {
            self.cf_handle()
        }
    }
}

impl TrieDB for RocksDBTrieDB {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        self.last_computed_flatkeyvalue >= key
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let cf = self.cf_handle_for_key(&key)?;
        let db_key = self.make_key(key);

        let res = self
            .db
            .get_cf(&cf, &db_key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB get error: {}", e)))?;

        Ok(res)
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let mut batch = rocksdb::WriteBatch::default();

        for (key, value) in key_values {
            let cf = self.cf_handle_for_key(&key)?;
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

    fn put_batch_no_alloc(&self, key_values: &[(Nibbles, Node)]) -> Result<(), TrieError> {
        let mut batch = rocksdb::WriteBatch::default();

        // 532 is the maximum size of an encoded branch node.
        let mut buffer = Vec::with_capacity(532);

        for (hash, node) in key_values {
            let cf = self.cf_handle_for_key(hash)?;
            let db_key = self.make_key(hash.clone());
            buffer.clear();
            node.encode(&mut buffer);
            batch.put_cf(&cf, db_key, &buffer);
        }

        self.db
            .write(batch)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB batch write error: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_trie::Nibbles;
    use rocksdb::{ColumnFamilyDescriptor, MultiThreaded, Options};
    use tempfile::TempDir;

    #[test]
    fn test_trie_db_basic_operations() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_db");

        // Setup RocksDB with column family
        let mut db_options = Options::default();
        db_options.create_if_missing(true);
        db_options.create_missing_column_families(true);

        let cf_trie = ColumnFamilyDescriptor::new("test_trie_cf", Options::default());
        let cf_fkv = ColumnFamilyDescriptor::new("test_flatkey_cf", Options::default());
        let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
            &db_options,
            db_path,
            vec![cf_trie, cf_fkv],
        )
        .unwrap();
        let db = Arc::new(db);

        // Create TrieDB
        let trie_db =
            RocksDBTrieDB::new(db, "test_trie_cf", "test_flatkey_cf", None, vec![]).unwrap();

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

        let cf_trie = ColumnFamilyDescriptor::new("test_trie_cf", Options::default());
        let cf_fkv = ColumnFamilyDescriptor::new("test_flatkey_cf", Options::default());
        let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
            &db_options,
            db_path,
            vec![cf_trie, cf_fkv],
        )
        .unwrap();
        let db = Arc::new(db);

        // Create TrieDB with address prefix
        let address = H256::from([0xaa; 32]);
        let trie_db =
            RocksDBTrieDB::new(db, "test_trie_cf", "test_flatkey_cf", Some(address), vec![])
                .unwrap();

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

        let cf_trie = ColumnFamilyDescriptor::new("test_trie_cf", Options::default());
        let cf_fkv = ColumnFamilyDescriptor::new("test_flatkey_cf", Options::default());
        let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
            &db_options,
            db_path,
            vec![cf_trie, cf_fkv],
        )
        .unwrap();
        let db = Arc::new(db);

        // Create TrieDB
        let trie_db =
            RocksDBTrieDB::new(db, "test_trie_cf", "test_flatkey_cf", None, vec![]).unwrap();

        // Test data
        // NOTE: we don't use the same paths to avoid overwriting in the batch
        let batch_data = vec![
            (Nibbles::from_hex(vec![1]), vec![1, 2, 3]),
            (Nibbles::from_hex(vec![1, 2]), vec![4, 5, 6]),
            (Nibbles::from_hex(vec![1, 2, 3]), vec![7, 8, 9]),
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
