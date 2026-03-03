//! EthrexDb storage backend — routes state/storage to ethrex-db, metadata to RocksDB.
//!
//! Trie-related tables (ACCOUNT_TRIE_NODES, STORAGE_TRIE_NODES, ACCOUNT_FLATKEYVALUE,
//! STORAGE_FLATKEYVALUE) are handled by ethrex-db (which computes Merkle tries internally).
//! All other tables (headers, bodies, receipts, chain data, etc.) are stored in RocksDB.

use std::path::Path;
use std::sync::{Arc, RwLock};

use crate::api::tables::{
    ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES,
};
use crate::api::{
    PrefixResult, StorageBackend, StorageLockedView, StorageReadView, StorageWriteBatch,
};
use crate::error::StoreError;

use super::rocksdb::RocksDBBackend;

/// Returns true if the table is managed by ethrex-db rather than RocksDB.
fn is_ethrex_db_table(table: &str) -> bool {
    matches!(
        table,
        ACCOUNT_TRIE_NODES | STORAGE_TRIE_NODES | ACCOUNT_FLATKEYVALUE | STORAGE_FLATKEYVALUE
    )
}

/// Hybrid backend: ethrex-db for state/storage tries, RocksDB for chain metadata.
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
        let paged_db = ethrex_db::store::PagedDb::open(ethrex_db_path).map_err(|e| {
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
        if is_ethrex_db_table(table) {
            // ethrex-db manages trie data internally; clearing individual tables
            // is not supported. This is a no-op since trie state is rebuilt on
            // finalization.
            Ok(())
        } else {
            self.metadata_db.clear_table(table)
        }
    }

    fn begin_read(&self) -> Result<Arc<dyn StorageReadView>, StoreError> {
        let metadata_view = self.metadata_db.begin_read()?;
        Ok(Arc::new(EthrexDbReadView {
            blockchain: self.blockchain.clone(),
            metadata_view,
        }))
    }

    fn begin_write(&self) -> Result<Box<dyn StorageWriteBatch + 'static>, StoreError> {
        let metadata_batch = self.metadata_db.begin_write()?;
        Ok(Box::new(EthrexDbWriteBatch { metadata_batch }))
    }

    fn begin_locked(
        &self,
        table_name: &'static str,
    ) -> Result<Box<dyn StorageLockedView + 'static>, StoreError> {
        if is_ethrex_db_table(table_name) {
            // ethrex-db doesn't expose table-level snapshots.
            // Return a stub view that always returns None.
            Ok(Box::new(EthrexDbLockedView))
        } else {
            self.metadata_db.begin_locked(table_name)
        }
    }

    fn create_checkpoint(&self, path: &Path) -> Result<(), StoreError> {
        // Checkpoint the RocksDB metadata database.
        // ethrex-db state is persisted on finalization; no separate checkpoint needed.
        self.metadata_db.create_checkpoint(path)
    }
}

/// Read-only view that routes reads between ethrex-db and RocksDB.
struct EthrexDbReadView {
    /// Retained for future use when ethrex-db table reads are routed through the blockchain API.
    #[allow(dead_code)]
    blockchain: Arc<RwLock<ethrex_db::chain::Blockchain>>,
    metadata_view: Arc<dyn StorageReadView>,
}

// Safety: EthrexDbReadView holds Arc<RwLock<Blockchain>> where Blockchain may not
// implement Send/Sync, and Arc<dyn StorageReadView> (Send+Sync via trait bound).
unsafe impl Send for EthrexDbReadView {}
unsafe impl Sync for EthrexDbReadView {}

impl StorageReadView for EthrexDbReadView {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        if is_ethrex_db_table(table) {
            // ethrex-db doesn't store individual trie nodes or flat key-values
            // in a way that's accessible by raw key lookup at the StorageBackend level.
            // Real state access is routed at the Store method level (Tasks 3A-3D).
            // Returning None here is safe — the Store layer will query the blockchain
            // directly for account/storage data.
            Ok(None)
        } else {
            self.metadata_view.get(table, key)
        }
    }

    fn prefix_iterator(
        &self,
        table: &'static str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError> {
        if is_ethrex_db_table(table) {
            // ethrex-db doesn't support prefix iteration over trie internals.
            // Return an empty iterator.
            Ok(Box::new(std::iter::empty()))
        } else {
            self.metadata_view.prefix_iterator(table, prefix)
        }
    }
}

/// Write batch that routes writes between ethrex-db and RocksDB.
struct EthrexDbWriteBatch {
    metadata_batch: Box<dyn StorageWriteBatch + 'static>,
}

// Safety: EthrexDbWriteBatch holds Box<dyn StorageWriteBatch> (Send).
unsafe impl Send for EthrexDbWriteBatch {}

impl StorageWriteBatch for EthrexDbWriteBatch {
    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        if is_ethrex_db_table(table) {
            // Trie writes are handled by ethrex-db internally during finalization.
            // The Store layer (Tasks 3A-3D) will call the blockchain API directly
            // for state mutations. Silently drop raw trie puts.
            Ok(())
        } else {
            self.metadata_batch.put(table, key, value)
        }
    }

    fn put_batch(
        &mut self,
        table: &'static str,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        if is_ethrex_db_table(table) {
            Ok(())
        } else {
            self.metadata_batch.put_batch(table, batch)
        }
    }

    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        if is_ethrex_db_table(table) {
            Ok(())
        } else {
            self.metadata_batch.delete(table, key)
        }
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        // Commit the RocksDB metadata writes.
        // ethrex-db state is committed through the blockchain API during finalization.
        self.metadata_batch.commit()
    }
}

/// Locked view for ethrex-db tables.
///
/// ethrex-db doesn't expose raw trie nodes at the key-value level,
/// so this view always returns None. The snap sync layer will need
/// to query the blockchain API directly when using ethrex-db.
struct EthrexDbLockedView;

impl StorageLockedView for EthrexDbLockedView {
    fn get(&self, _key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        // Locked views are used during snap sync for trie traversal.
        // ethrex-db doesn't expose raw trie nodes at the key-value level.
        // The snap sync layer will need to be adapted to query the blockchain
        // API directly when using ethrex-db.
        Ok(None)
    }
}
