use ethrex_common::H256;
use ethrex_trie::{NodeHash, TrieDB, error::TrieError};
use rocksdb::{DBWithThreadMode, MultiThreaded, SnapshotWithThreadMode};
use smallvec::SmallVec;
use std::sync::Arc;

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
    fn get(
        &self,
        prefix_len: usize,
        full_path: SmallVec<[u8; 32]>,
        node_hash: NodeHash,
    ) -> Result<Option<Vec<u8>>, TrieError> {
        let db_key = self.make_key(prefix_len, full_path, node_hash);

        self.snapshot
            .get_cf(&self.cf, db_key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("RocksDB snapshot get error: {}", e)))
    }

    fn put_batch(
        &self,
        key_values: Vec<(usize, SmallVec<[u8; 32]>, NodeHash, Vec<u8>)>,
    ) -> Result<(), TrieError> {
        Err(TrieError::DbError(anyhow::anyhow!(
            "LockedTrie is read-only"
        )))
    }
}
