use ethrex_common::H256;
use ethrex_trie::{Nibbles, TrieDB, error::TrieError};
use rocksdb::{DBWithThreadMode, MultiThreaded, SnapshotWithThreadMode};
use std::sync::Arc;

use crate::trie_db::layering::apply_prefix;

/// RocksDB locked implementation for the TrieDB trait, read-only with consistent snapshot.
pub struct RocksDBLockedTrieDB {
    /// RocksDB database
    db: &'static Arc<DBWithThreadMode<MultiThreaded>>,
    /// Column family handle
    cf: std::sync::Arc<rocksdb::BoundColumnFamily<'static>>,
    /// Read-only snapshot for consistent reads
    snapshot: SnapshotWithThreadMode<'static, DBWithThreadMode<MultiThreaded>>,
    /// Storage trie address prefix
    address_prefix: Option<H256>,
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

        // Create snapshot for consistent reads
        let snapshot = db.snapshot();

        Ok(Self {
            db,
            cf,
            snapshot,
            address_prefix,
        })
    }

    fn make_key(&self, node_hash: Nibbles) -> Vec<u8> {
        apply_prefix(self.address_prefix, node_hash).as_ref().to_vec()
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
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let db_key = self.make_key(key);

        self.snapshot
            .get_cf(&self.cf, db_key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB snapshot get error: {}", e)))
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        Err(TrieError::DbError(anyhow::anyhow!(
            "LockedTrie is read-only"
        )))
    }
}
