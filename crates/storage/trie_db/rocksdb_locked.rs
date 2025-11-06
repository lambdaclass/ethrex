use canopydb::{Database, ReadTransaction};
use ethrex_common::H256;
use ethrex_trie::{Nibbles, TrieDB, error::TrieError};
use std::sync::{Arc, Mutex};

use crate::{
    store_db::rocksdb::{CF_FLATKEYVALUE, CF_TRIE_NODES},
    trie_db::layering::apply_prefix,
};

const TX_POOL_SIZE: usize = 16;

/// RocksDB locked implementation for the TrieDB trait, read-only with consistent snapshot.
pub struct RocksDBLockedTrieDB {
    /// Read-only snapshot for consistent reads
    read_txs: [Mutex<ReadTransaction>; TX_POOL_SIZE],
    /// Storage trie address prefix
    address_prefix: Option<H256>,
    last_computed_flatkeyvalue: Nibbles,
}

impl RocksDBLockedTrieDB {
    pub fn new(
        db: Arc<Database>,
        address_prefix: Option<H256>,
        last_written: Vec<u8>,
    ) -> Result<Self, TrieError> {
        let read_txs = std::array::from_fn(|_| Mutex::new(db.begin_read().unwrap()));

        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);

        Ok(Self {
            read_txs,
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

impl TrieDB for RocksDBLockedTrieDB {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        self.last_computed_flatkeyvalue >= key
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let cf = if key.is_leaf() {
            CF_FLATKEYVALUE.as_bytes()
        } else {
            CF_TRIE_NODES.as_bytes()
        };
        let db_key = self.make_key(key);

        let opt_read_tx = self.read_txs.iter().find_map(|tx| tx.try_lock().ok());
        let read_tx = opt_read_tx
            .or_else(|| Some(self.read_txs.first().unwrap().lock().unwrap()))
            .unwrap();

        read_tx
            .get_tree(cf)
            .unwrap()
            .unwrap()
            .get(&db_key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB snapshot get error: {}", e)))
            .map(|o| o.map(|b| b.to_vec()))
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        Err(TrieError::DbError(anyhow::anyhow!(
            "LockedTrie is read-only"
        )))
    }
}
