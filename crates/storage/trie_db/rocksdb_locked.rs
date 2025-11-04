use canopydb::{Database, ReadTransaction};
use ethrex_common::H256;
use ethrex_trie::{Nibbles, TrieDB, error::TrieError};
use std::{collections::HashMap, sync::Arc};

use crate::{store_db::rocksdb::CF_FLATKEYVALUE, trie_db::layering::apply_prefix};

/// RocksDB locked implementation for the TrieDB trait, read-only with consistent snapshot.
pub struct RocksDBLockedTrieDB {
    /// Column family handle
    dbs: Arc<HashMap<String, Database>>,
    /// Read-only snapshot for consistent reads
    snapshot: ReadTransaction,
    snapshot_fkv: ReadTransaction,
    /// Storage trie address prefix
    address_prefix: Option<H256>,
    last_computed_flatkeyvalue: Nibbles,
}

impl RocksDBLockedTrieDB {
    pub fn new(
        dbs: Arc<HashMap<String, Database>>,
        cf_name: &str,
        address_prefix: Option<H256>,
        last_written: Vec<u8>,
    ) -> Result<Self, TrieError> {
        let snapshot = dbs.get(cf_name).unwrap().begin_read().unwrap();
        let snapshot_fkv = dbs.get(CF_FLATKEYVALUE).unwrap().begin_read().unwrap();

        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);

        Ok(Self {
            dbs,
            snapshot,
            snapshot_fkv,
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
            &self.snapshot_fkv
        } else {
            &self.snapshot
        };
        let db_key = self.make_key(key);

        self.snapshot
            .get_tree(b"")
            .unwrap()
            .unwrap()
            .get(db_key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB snapshot get error: {}", e)))
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        Err(TrieError::DbError(anyhow::anyhow!(
            "LockedTrie is read-only"
        )))
    }
}
