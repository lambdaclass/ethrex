use canopydb::{Database, Environment};
use ethrex_common::H256;
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::{Nibbles, Node, TrieDB, error::TrieError};
use std::{collections::BTreeMap, sync::Arc};

use crate::{store_db::rocksdb::CF_FLATKEYVALUE, trie_db::layering::apply_prefix};

/// RocksDB implementation for the TrieDB trait, with get and put operations.
pub struct RocksDBTrieDB {
    /// RocksDB database
    db: Environment,
    /// RocksDB database
    dbs: Arc<BTreeMap<String, Database>>,
    /// Column family name
    cf_name: String,
    /// Storage trie address prefix
    address_prefix: Option<H256>,
    /// Last flatkeyvalue path already generated
    last_computed_flatkeyvalue: Nibbles,
}

impl RocksDBTrieDB {
    pub fn new(
        db: Environment,
        dbs: Arc<BTreeMap<String, Database>>,
        cf_name: &str,
        address_prefix: Option<H256>,
        last_written: Vec<u8>,
    ) -> Result<Self, TrieError> {
        // Verify column family exists
        if dbs.get(cf_name).is_none() {
            return Err(TrieError::DbError(anyhow::anyhow!(
                "Column family not found: {}",
                cf_name
            )));
        }
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);

        Ok(Self {
            db,
            dbs,
            cf_name: cf_name.to_string(),
            address_prefix,
            last_computed_flatkeyvalue,
        })
    }

    fn cf_handle(&self) -> Result<&Database, TrieError> {
        self.dbs
            .get(&self.cf_name)
            .ok_or_else(|| TrieError::DbError(anyhow::anyhow!("Column family not found")))
    }

    fn cf_handle_flatkeyvalue(&self) -> Result<&Database, TrieError> {
        self.dbs
            .get(CF_FLATKEYVALUE)
            .ok_or_else(|| TrieError::DbError(anyhow::anyhow!("Column family not found")))
    }

    fn make_key(&self, node_hash: Nibbles) -> Vec<u8> {
        apply_prefix(self.address_prefix, node_hash)
            .as_ref()
            .to_vec()
    }
}

impl TrieDB for RocksDBTrieDB {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        self.last_computed_flatkeyvalue >= key
    }
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let cf = if key.is_leaf() {
            self.cf_handle_flatkeyvalue()?
        } else {
            self.cf_handle()?
        };
        let db_key = self.make_key(key);

        let res = cf
            .begin_read()
            .unwrap()
            .get_tree(b"")
            .unwrap()
            .unwrap()
            .get(&db_key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB get error: {}", e)))?
            .map(|b| b.to_vec());
        Ok(res)
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let cf = self.cf_handle()?;
        let cf_snapshot = self.cf_handle_flatkeyvalue()?;

        let cf_snapshot_tx = cf_snapshot.begin_write_concurrent().unwrap();
        let cf_tx = cf.begin_write_concurrent().unwrap();
        {
            let mut tree = cf_tx.get_tree(b"").unwrap().unwrap();
            let mut snapshot_tree = cf_snapshot_tx.get_tree(b"").unwrap().unwrap();
            for (key, value) in key_values {
                let cf = if key.is_leaf() {
                    &mut snapshot_tree
                } else {
                    &mut tree
                };
                let db_key = self.make_key(key);
                if value.is_empty() {
                    cf.delete(&db_key).unwrap();
                } else {
                    cf.insert(&db_key, &value).unwrap();
                }
            }
        }
        self.db
            .group_commit([cf_snapshot_tx, cf_tx], false)
            .unwrap();
        Ok(())
    }

    fn put_batch_no_alloc(&self, key_values: &[(Nibbles, Node)]) -> Result<(), TrieError> {
        let cf = self.cf_handle()?;
        let cf_flatkeyvalue = self.cf_handle_flatkeyvalue()?;

        let cf_flatkeyvalue_tx = cf_flatkeyvalue.begin_write_concurrent().unwrap();
        let cf_tx = cf.begin_write_concurrent().unwrap();

        // 532 is the maximum size of an encoded branch node.
        let mut buffer = Vec::with_capacity(532);

        {
            let mut tree = cf_tx.get_tree(b"").unwrap().unwrap();
            let mut flatkeyvalue_tree = cf_flatkeyvalue_tx.get_tree(b"").unwrap().unwrap();
            for (hash, node) in key_values {
                let cf = if hash.is_leaf() {
                    &mut flatkeyvalue_tree
                } else {
                    &mut tree
                };
                let db_key = self.make_key(hash.clone());
                buffer.clear();
                node.encode(&mut buffer);
                cf.insert(&db_key, &buffer).unwrap();
            }
        }

        self.db
            .group_commit([cf_flatkeyvalue_tx, cf_tx], false)
            .unwrap();

        Ok(())
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use ethrex_trie::Nibbles;
//     use rocksdb::{ColumnFamilyDescriptor, MultiThreaded, Options};
//     use tempfile::TempDir;

//     #[test]
//     fn test_trie_db_basic_operations() {
//         let temp_dir = TempDir::new().unwrap();
//         let db_path = temp_dir.path().join("test_db");

//         // Setup RocksDB with column family
//         let mut db_options = Options::default();
//         db_options.create_if_missing(true);
//         db_options.create_missing_column_families(true);

//         let cf_descriptor = ColumnFamilyDescriptor::new("test_cf", Options::default());
//         let cf_fkv = ColumnFamilyDescriptor::new(CF_FLATKEYVALUE, Options::default());
//         let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
//             &db_options,
//             db_path,
//             vec![cf_descriptor, cf_fkv],
//         )
//         .unwrap();
//         let db = Arc::new(db);

//         // Create TrieDB
//         let trie_db = RocksDBTrieDB::new(db, "test_cf", None, vec![]).unwrap();

//         // Test data
//         let node_hash = Nibbles::from_hex(vec![1]);
//         let node_data = vec![1, 2, 3, 4, 5];

//         // Test put_batch
//         trie_db
//             .put_batch(vec![(node_hash.clone(), node_data.clone())])
//             .unwrap();

//         // Test get
//         let retrieved_data = trie_db.get(node_hash).unwrap().unwrap();
//         assert_eq!(retrieved_data, node_data);

//         // Test get nonexistent
//         let nonexistent_hash = Nibbles::from_hex(vec![2]);
//         assert!(trie_db.get(nonexistent_hash).unwrap().is_none());
//     }

//     #[test]
//     fn test_trie_db_with_address_prefix() {
//         let temp_dir = TempDir::new().unwrap();
//         let db_path = temp_dir.path().join("test_db");

//         // Setup RocksDB with column family
//         let mut db_options = Options::default();
//         db_options.create_if_missing(true);
//         db_options.create_missing_column_families(true);

//         let cf_descriptor = ColumnFamilyDescriptor::new("test_cf", Options::default());
//         let cf_fkv = ColumnFamilyDescriptor::new(CF_FLATKEYVALUE, Options::default());
//         let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
//             &db_options,
//             db_path,
//             vec![cf_descriptor, cf_fkv],
//         )
//         .unwrap();
//         let db = Arc::new(db);

//         // Create TrieDB with address prefix
//         let address = H256::from([0xaa; 32]);
//         let trie_db = RocksDBTrieDB::new(db, "test_cf", Some(address), vec![]).unwrap();

//         // Test data
//         let node_hash = Nibbles::from_hex(vec![1]);
//         let node_data = vec![1, 2, 3, 4, 5];

//         // Test put_batch
//         trie_db
//             .put_batch(vec![(node_hash.clone(), node_data.clone())])
//             .unwrap();

//         // Test get
//         let retrieved_data = trie_db.get(node_hash).unwrap().unwrap();
//         assert_eq!(retrieved_data, node_data);
//     }

//     #[test]
//     fn test_trie_db_batch_operations() {
//         let temp_dir = TempDir::new().unwrap();
//         let db_path = temp_dir.path().join("test_db");

//         // Setup RocksDB with column family
//         let mut db_options = Options::default();
//         db_options.create_if_missing(true);
//         db_options.create_missing_column_families(true);

//         let cf_descriptor = ColumnFamilyDescriptor::new("test_cf", Options::default());
//         let cf_fkv = ColumnFamilyDescriptor::new(CF_FLATKEYVALUE, Options::default());
//         let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
//             &db_options,
//             db_path,
//             vec![cf_descriptor, cf_fkv],
//         )
//         .unwrap();
//         let db = Arc::new(db);

//         // Create TrieDB
//         let trie_db = RocksDBTrieDB::new(db, "test_cf", None, vec![]).unwrap();

//         // Test data
//         // NOTE: we don't use the same paths to avoid overwriting in the batch
//         let batch_data = vec![
//             (Nibbles::from_hex(vec![1]), vec![1, 2, 3]),
//             (Nibbles::from_hex(vec![1, 2]), vec![4, 5, 6]),
//             (Nibbles::from_hex(vec![1, 2, 3]), vec![7, 8, 9]),
//         ];

//         // Test batch put
//         trie_db.put_batch(batch_data.clone()).unwrap();

//         // Test batch get
//         for (node_hash, expected_data) in batch_data {
//             let retrieved_data = trie_db.get(node_hash).unwrap().unwrap();
//             assert_eq!(retrieved_data, expected_data);
//         }
//     }
// }
