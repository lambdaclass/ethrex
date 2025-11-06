use canopydb::Database;
use ethrex_common::H256;
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::{Nibbles, Node, TrieDB, error::TrieError};
use std::sync::Arc;

use crate::{
    error::StoreError,
    store_db::rocksdb::{CF_FLATKEYVALUE, CF_TRIE_NODES, WriterMessage},
    trie_db::layering::apply_prefix,
};

/// RocksDB implementation for the TrieDB trait, with get and put operations.
pub struct RocksDBTrieDB {
    /// RocksDB database
    db: Arc<Database>,
    writer_tx: std::sync::mpsc::SyncSender<(
        WriterMessage,
        std::sync::mpsc::SyncSender<Result<(), StoreError>>,
    )>,
    /// Column family name
    cf_name: String,
    /// Storage trie address prefix
    address_prefix: Option<H256>,
    /// Last flatkeyvalue path already generated
    last_computed_flatkeyvalue: Nibbles,
}

impl RocksDBTrieDB {
    pub fn new(
        db: Arc<Database>,
        writer_tx: std::sync::mpsc::SyncSender<(
            WriterMessage,
            std::sync::mpsc::SyncSender<Result<(), StoreError>>,
        )>,
        cf_name: &str,
        address_prefix: Option<H256>,
        last_written: Vec<u8>,
    ) -> Result<Self, TrieError> {
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);

        Ok(Self {
            db,
            writer_tx,
            cf_name: cf_name.to_string(),
            address_prefix,
            last_computed_flatkeyvalue,
        })
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
        let tx = self.db.begin_read().unwrap();
        let cf = if key.is_leaf() {
            tx.get_tree(CF_FLATKEYVALUE.as_bytes()).unwrap().unwrap()
        } else {
            tx.get_tree(self.cf_name.as_bytes()).unwrap().unwrap()
        };
        let db_key = self.make_key(key);

        let res = cf
            .get(&db_key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB get error: {}", e)))?
            .map(|b| b.to_vec());
        Ok(res)
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let mut batch_ops = Vec::with_capacity(key_values.len());

        for (key, value) in key_values {
            let cf = if key.is_leaf() {
                CF_FLATKEYVALUE
            } else {
                CF_TRIE_NODES
            };
            let db_key = self.make_key(key);
            if value.is_empty() {
                batch_ops.push((cf, db_key, vec![]));
            } else {
                batch_ops.push((cf, db_key.clone(), value));
            }
        }
        let msg = WriterMessage::WriteBatchAsync { batch_ops };
        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        self.writer_tx.send((msg, tx)).unwrap();
        rx.recv().unwrap().unwrap();
        Ok(())
    }

    fn put_batch_no_alloc(&self, key_values: &[(Nibbles, Node)]) -> Result<(), TrieError> {
        let mut batch_ops = Vec::with_capacity(key_values.len());

        for (key, node) in key_values {
            let cf = if key.is_leaf() {
                CF_FLATKEYVALUE
            } else {
                CF_TRIE_NODES
            };
            let db_key = self.make_key(key.clone());
            batch_ops.push((cf, db_key, node.encode_to_vec()));
        }
        let msg = WriterMessage::WriteBatchAsync { batch_ops };
        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        self.writer_tx.send((msg, tx)).unwrap();
        rx.recv().unwrap().unwrap();
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
