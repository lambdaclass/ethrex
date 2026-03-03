//! EthrexDb storage backend — hybrid backend with ethrex-db for state management
//! and RocksDB for all table-level storage (including trie tables).
//!
//! ALL table reads/writes go to RocksDB. This ensures snap sync's trie building
//! and healing work correctly (they need raw trie node read/write at the table level).
//!
//! State access during block execution is routed through the ethrex-db Blockchain API
//! at the Store method level (get_account_info_by_hash, get_storage_at, etc.),
//! not through these table operations.
//!
//! After snap sync, state is transferred from the RocksDB trie into ethrex-db
//! via Store::transfer_snap_state_to_ethrex_db().

use std::path::Path;
use std::sync::{Arc, RwLock};

use crate::api::{StorageBackend, StorageLockedView, StorageReadView, StorageWriteBatch};
use crate::error::StoreError;

use super::rocksdb::RocksDBBackend;

/// Hybrid backend: ethrex-db for state management, RocksDB for all table storage.
pub struct EthrexDbBackend {
    pub blockchain: Arc<RwLock<ethrex_db::chain::Blockchain>>,
    pub metadata_db: Arc<RocksDBBackend>,
}

impl std::fmt::Debug for EthrexDbBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EthrexDbBackend")
            .field("metadata_db", &self.metadata_db)
            .finish_non_exhaustive()
    }
}

impl EthrexDbBackend {
    /// Opens a new hybrid backend.
    ///
    /// - `ethrex_db_path`: directory for the ethrex-db state database.
    /// - `rocksdb_path`: directory for the RocksDB chain metadata database.
    pub fn new(ethrex_db_path: &Path, rocksdb_path: &Path) -> Result<Self, StoreError> {
        // 32GB initial size (8M pages * 4KB) to fit full Ethereum state
        let paged_db =
            ethrex_db::store::PagedDb::open_with_size(ethrex_db_path, 8_000_000).map_err(|e| {
                StoreError::Custom(format!("Failed to open ethrex-db at {ethrex_db_path:?}: {e}"))
            })?;
        let blockchain = ethrex_db::chain::Blockchain::new(paged_db);
        let metadata_db = RocksDBBackend::open(rocksdb_path)?;

        Ok(Self {
            blockchain: Arc::new(RwLock::new(blockchain)),
            metadata_db: Arc::new(metadata_db),
        })
    }
}

impl StorageBackend for EthrexDbBackend {
    fn clear_table(&self, table: &'static str) -> Result<(), StoreError> {
        self.metadata_db.clear_table(table)
    }

    fn begin_read(&self) -> Result<Arc<dyn StorageReadView>, StoreError> {
        self.metadata_db.begin_read()
    }

    fn begin_write(&self) -> Result<Box<dyn StorageWriteBatch + 'static>, StoreError> {
        self.metadata_db.begin_write()
    }

    fn begin_locked(
        &self,
        table_name: &'static str,
    ) -> Result<Box<dyn StorageLockedView + 'static>, StoreError> {
        self.metadata_db.begin_locked(table_name)
    }

    fn create_checkpoint(&self, path: &Path) -> Result<(), StoreError> {
        self.metadata_db.create_checkpoint(path)
    }
}
