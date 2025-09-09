use ethrex_common::H256;
use ethrex_trie::{NodeHash, TrieDB, error::TrieError};
use rocksdb::{DBWithThreadMode, MultiThreaded};
use smallvec::SmallVec;
use std::sync::Arc;

/// RocksDB implementation for the TrieDB trait, with get and put operations.
pub struct RocksDBTrieDB {
    /// RocksDB database
    db: Arc<DBWithThreadMode<MultiThreaded>>,
    /// Column family name
    cf_name: String,
    /// Storage trie address prefix
    address_prefix: Option<H256>,
}

impl RocksDBTrieDB {
    pub fn new(
        db: Arc<DBWithThreadMode<MultiThreaded>>,
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

    fn make_key(
        &self,
        prefix_len: usize,
        full_path: SmallVec<[u8; 32]>,
        node_hash: NodeHash,
    ) -> Vec<u8> {
        match &self.address_prefix {
            Some(address) => {
                // For storage tries, prefix with address
                let mut key = [0u8; 97];
                key[..32].copy_from_slice(&address.0);
                let to_copy = prefix_len.div_ceil(2);
                key[32..32 + to_copy].copy_from_slice(&full_path[..to_copy]);
                if prefix_len % 2 != 0 {
                    key[32 + to_copy - 1] &= 0xf0;
                }
                key[64] = prefix_len as u8;
                key[65..].copy_from_slice(&node_hash.finalize().0);
                key.to_vec()
            }
            None => {
                // For state trie, use node hash directly
                let mut key = [0u8; 65];
                let to_copy = prefix_len.div_ceil(2);
                key[..to_copy].copy_from_slice(&full_path[..to_copy]);
                if prefix_len % 2 != 0 {
                    key[to_copy - 1] &= 0xf0;
                }
                key[32] = prefix_len as u8;
                key[33..].copy_from_slice(&node_hash.finalize().0);
                key.to_vec()
            }
        }
    }
}

impl TrieDB for RocksDBTrieDB {
    fn get(
        &self,
        prefix_len: usize,
        full_path: SmallVec<[u8; 32]>,
        node_hash: NodeHash,
    ) -> Result<Option<Vec<u8>>, TrieError> {
        let cf = self.cf_handle()?;
        let db_key = self.make_key(prefix_len, full_path, node_hash);

        self.db
            .get_cf(&cf, db_key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB get error: {}", e)))
    }

    fn put_batch(
        &self,
        key_values: Vec<(usize, SmallVec<[u8; 32]>, NodeHash, Vec<u8>)>,
    ) -> Result<(), TrieError> {
        let cf = self.cf_handle()?;
        let mut batch = rocksdb::WriteBatch::default();

        for (prefix_len, full_path, node_hash, value) in key_values {
            let db_key = self.make_key(prefix_len, full_path, node_hash);
            batch.put_cf(&cf, db_key, value);
        }

        self.db
            .write(batch)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB batch write error: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_trie::NodeHash;
    use rocksdb::{ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded, Options};
    use tempdir::TempDir;

    #[test]
    fn test_trie_db_basic_operations() {
        let temp_dir = TempDir::new("rocksdb_trie_test").unwrap();
        let db_path = temp_dir.path().join("test_db");

        // Setup RocksDB with column family
        let mut db_options = Options::default();
        db_options.create_if_missing(true);
        db_options.create_missing_column_families(true);

        let cf_descriptor = ColumnFamilyDescriptor::new("test_cf", Options::default());
        let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
            &db_options,
            db_path,
            vec![cf_descriptor],
        )
        .unwrap();
        let db = Arc::new(db);

        // Create TrieDB
        let trie_db = RocksDBTrieDB::new(db, "test_cf", None).unwrap();

        // Test data
        let node_hash = NodeHash::from(H256::from([1u8; 32]));
        let node_data = vec![1, 2, 3, 4, 5];

        // Test put_batch
        trie_db
            .put_batch(vec![(0, SmallVec::new(), node_hash, node_data.clone())])
            .unwrap();

        // Test get
        let retrieved_data = trie_db.get(0, SmallVec::new(), node_hash).unwrap().unwrap();
        assert_eq!(retrieved_data, node_data);

        // Test get nonexistent
        let nonexistent_hash = NodeHash::from(H256::from([2u8; 32]));
        assert!(
            trie_db
                .get(0, SmallVec::new(), nonexistent_hash)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn test_trie_db_with_address_prefix() {
        let temp_dir = TempDir::new("rocksdb_trie_test").unwrap();
        let db_path = temp_dir.path().join("test_db");

        // Setup RocksDB with column family
        let mut db_options = Options::default();
        db_options.create_if_missing(true);
        db_options.create_missing_column_families(true);

        let cf_descriptor = ColumnFamilyDescriptor::new("test_cf", Options::default());
        let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
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
        let node_hash = NodeHash::from(H256::from([1u8; 32]));
        let node_data = vec![1, 2, 3, 4, 5];

        // Test put_batch
        trie_db
            .put_batch(vec![(0, SmallVec::new(), node_hash, node_data.clone())])
            .unwrap();

        // Test get
        let retrieved_data = trie_db.get(0, SmallVec::new(), node_hash).unwrap().unwrap();
        assert_eq!(retrieved_data, node_data);
    }

    #[test]
    fn test_trie_db_batch_operations() {
        let temp_dir = TempDir::new("rocksdb_trie_test").unwrap();
        let db_path = temp_dir.path().join("test_db");

        // Setup RocksDB with column family
        let mut db_options = Options::default();
        db_options.create_if_missing(true);
        db_options.create_missing_column_families(true);

        let cf_descriptor = ColumnFamilyDescriptor::new("test_cf", Options::default());
        let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
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
            (
                0,
                SmallVec::new(),
                NodeHash::from(H256::from([1u8; 32])),
                vec![1, 2, 3],
            ),
            (
                0,
                SmallVec::new(),
                NodeHash::from(H256::from([2u8; 32])),
                vec![4, 5, 6],
            ),
            (
                0,
                SmallVec::new(),
                NodeHash::from(H256::from([3u8; 32])),
                vec![7, 8, 9],
            ),
        ];

        // Test batch put
        trie_db.put_batch(batch_data.clone()).unwrap();

        // Test batch get
        for (prefix_len, full_path, node_hash, expected_data) in batch_data {
            let retrieved_data = trie_db
                .get(prefix_len, full_path, node_hash)
                .unwrap()
                .unwrap();
            assert_eq!(retrieved_data, expected_data);
        }
    }
}
