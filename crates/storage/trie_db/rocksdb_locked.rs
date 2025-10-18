use ethrex_common::H256;
use ethrex_trie::{Nibbles, TrieDB, error::TrieError};
use rocksdb::{DBWithThreadMode, MultiThreaded, SnapshotWithThreadMode};
use std::sync::Arc;

use crate::{
    store_db::rocksdb::{CF_FLATKEYVALUE, CF_MISC_VALUES},
    trie_db::layering::apply_prefix,
};

/// RocksDB locked implementation for the TrieDB trait, read-only with consistent snapshot.
pub struct RocksDBLockedTrieDB {
    /// RocksDB database
    db: &'static Arc<DBWithThreadMode<MultiThreaded>>,
    /// Column family handle
    cf: std::sync::Arc<rocksdb::BoundColumnFamily<'static>>,
    /// Column family handle
    cf_flatkeyvalue: std::sync::Arc<rocksdb::BoundColumnFamily<'static>>,
    /// Read-only snapshot for consistent reads
    snapshot: SnapshotWithThreadMode<'static, DBWithThreadMode<MultiThreaded>>,
    /// Storage trie address prefix
    address_prefix: Option<H256>,
    last_computed_flatkeyvalue: Nibbles,
}

impl RocksDBLockedTrieDB {
    pub fn new(
        db: Arc<DBWithThreadMode<MultiThreaded>>,
        cf_name: &str,
        address_prefix: Option<H256>,
    ) -> Result<Self, TrieError> {
        // Leak the database reference to get 'static lifetime
        let db = Box::leak(Box::new(db));

        // Verify column family exists
        let cf = db.cf_handle(cf_name).ok_or_else(|| {
            TrieError::DbError(anyhow::anyhow!("Column family not found: {}", cf_name))
        })?;
        // Verify column family exists
        let cf_flatkeyvalue = db.cf_handle(CF_FLATKEYVALUE).ok_or_else(|| {
            TrieError::DbError(anyhow::anyhow!("Column family not found: {}", cf_name))
        })?;

        let cf_misc = db
            .cf_handle(CF_MISC_VALUES)
            .ok_or_else(|| TrieError::DbError(anyhow::anyhow!("Column family not found")))?;
        let last_computed_flatkeyvalue = db
            .get_cf(&cf_misc, "last_written")
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Error reading last_written: {e}")))?
            .map(|v| Nibbles::from_hex(v.to_vec()))
            .unwrap_or_default();
        drop(cf_misc);

        // Create snapshot for consistent reads
        let snapshot = db.snapshot();

        Ok(Self {
            db,
            cf,
            cf_flatkeyvalue,
            snapshot,
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

impl Drop for RocksDBLockedTrieDB {
    fn drop(&mut self) {
        // Restore the leaked database reference
        unsafe {
            drop(Box::from_raw(
                self.db as *const Arc<DBWithThreadMode<MultiThreaded>>
                    as *mut Arc<DBWithThreadMode<MultiThreaded>>,
            ));
        }
    }
}

impl TrieDB for RocksDBLockedTrieDB {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        self.last_computed_flatkeyvalue >= key
    }
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let cf = if key.is_leaf() {
            &self.cf_flatkeyvalue
        } else {
            &self.cf
        };
        let db_key = self.make_key(key);

        self.snapshot
            .get_cf(cf, db_key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB snapshot get error: {}", e)))
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        Err(TrieError::DbError(anyhow::anyhow!(
            "LockedTrie is read-only"
        )))
    }
}
