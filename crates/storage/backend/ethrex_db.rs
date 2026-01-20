//! Hybrid backend using ethrex_db for state/storage tries and RocksDB for other data.
//!
//! This backend combines:
//! - ethrex_db's `Blockchain` for state trie and storage trie operations (hot + cold storage)
//! - RocksDB for blocks, headers, receipts, and other blockchain data
//!
//! The hybrid approach leverages ethrex_db's optimized trie storage (memory-mapped pages,
//! Copy-on-Write concurrency) while keeping RocksDB for data that benefits from its
//! compression and indexing capabilities.

use crate::api::tables::{
    ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES,
};
use crate::api::{
    PrefixResult, StorageBackend, StorageLockedView, StorageReadView, StorageWriteBatch,
};
use crate::error::StoreError;
use ethrex_db::chain::Blockchain;
use ethrex_db::store::PagedDb;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, RwLock};

use super::rocksdb::RocksDBBackend;

/// Tables that are handled by ethrex_db (state and storage tries).
const ETHREX_DB_TABLES: [&str; 4] = [
    ACCOUNT_TRIE_NODES,
    STORAGE_TRIE_NODES,
    ACCOUNT_FLATKEYVALUE,
    STORAGE_FLATKEYVALUE,
];

/// Check if a table should be routed to ethrex_db.
fn is_ethrex_db_table(table: &str) -> bool {
    ETHREX_DB_TABLES.contains(&table)
}

/// Hybrid backend combining ethrex_db and RocksDB.
///
/// State and storage trie operations are routed to ethrex_db's `Blockchain`,
/// while all other operations go to RocksDB.
pub struct EthrexDbBackend {
    /// ethrex_db blockchain for state/storage trie management.
    /// Handles hot (unfinalized) and cold (finalized) state storage.
    blockchain: Arc<RwLock<Blockchain>>,

    /// RocksDB backend for blocks, headers, receipts, and other data.
    auxiliary: Arc<RocksDBBackend>,

    /// In-memory cache for trie data written but not yet committed to ethrex_db.
    /// Maps table -> key -> value.
    /// This bridges the gap between ethrex's write batch model and ethrex_db's block model.
    pending_trie_writes: Arc<RwLock<HashMap<&'static str, HashMap<Vec<u8>, Vec<u8>>>>>,
}

impl fmt::Debug for EthrexDbBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EthrexDbBackend")
            .field("blockchain", &"<Blockchain>")
            .field("auxiliary", &self.auxiliary)
            .field("pending_trie_writes", &"<pending>")
            .finish()
    }
}

impl EthrexDbBackend {
    /// Opens or creates a hybrid backend at the given path.
    ///
    /// Creates:
    /// - `state.db` file for ethrex_db (PagedDb)
    /// - `auxiliary/` directory for RocksDB
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();

        // Create parent directory if needed
        std::fs::create_dir_all(path)
            .map_err(|e| StoreError::Custom(format!("Failed to create data directory: {}", e)))?;

        // Paths for the two storage components
        let state_path = path.join("state.db");
        let auxiliary_path = path.join("auxiliary");

        std::fs::create_dir_all(&auxiliary_path).map_err(|e| {
            StoreError::Custom(format!("Failed to create auxiliary directory: {}", e))
        })?;

        // Open PagedDb for state storage (expects a file path)
        let paged_db = PagedDb::open(&state_path)
            .map_err(|e| StoreError::Custom(format!("Failed to open PagedDb: {}", e)))?;

        // Create Blockchain on top of PagedDb
        let blockchain = Blockchain::new(paged_db);

        // Open RocksDB for auxiliary storage
        let auxiliary = RocksDBBackend::open(&auxiliary_path)?;

        Ok(Self {
            blockchain: Arc::new(RwLock::new(blockchain)),
            auxiliary: Arc::new(auxiliary),
            pending_trie_writes: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Returns a reference to the underlying blockchain for direct state operations.
    ///
    /// This is useful for higher-level operations that need to interact with
    /// ethrex_db's native API (e.g., forkchoice updates, state root computation).
    pub fn blockchain(&self) -> Arc<RwLock<Blockchain>> {
        self.blockchain.clone()
    }

    /// Returns a reference to the auxiliary RocksDB backend.
    pub fn auxiliary(&self) -> Arc<RocksDBBackend> {
        self.auxiliary.clone()
    }
}

impl StorageBackend for EthrexDbBackend {
    fn clear_table(&self, table: &'static str) -> Result<(), StoreError> {
        if is_ethrex_db_table(table) {
            // Clear pending writes for this table
            let mut pending = self.pending_trie_writes.write().map_err(|_| {
                StoreError::Custom("Failed to acquire write lock on pending trie writes".into())
            })?;
            pending.remove(table);
            // Note: We don't clear ethrex_db state here as it's managed via finalization
            Ok(())
        } else {
            self.auxiliary.clear_table(table)
        }
    }

    fn begin_read(&self) -> Result<Box<dyn StorageReadView + '_>, StoreError> {
        let aux_read = self.auxiliary.begin_read()?;
        let pending = self.pending_trie_writes.read().map_err(|_| {
            StoreError::Custom("Failed to acquire read lock on pending trie writes".into())
        })?;

        Ok(Box::new(EthrexDbReadView {
            blockchain: self.blockchain.clone(),
            auxiliary: aux_read,
            pending_snapshot: pending.clone(),
        }))
    }

    fn begin_write(&self) -> Result<Box<dyn StorageWriteBatch + 'static>, StoreError> {
        let aux_write = self.auxiliary.begin_write()?;

        Ok(Box::new(EthrexDbWriteBatch {
            pending_trie_writes: self.pending_trie_writes.clone(),
            trie_batch: HashMap::new(),
            auxiliary: aux_write,
        }))
    }

    fn begin_locked(
        &self,
        table_name: &'static str,
    ) -> Result<Box<dyn StorageLockedView + 'static>, StoreError> {
        if is_ethrex_db_table(table_name) {
            // For trie tables, create a locked view that reads from pending writes + blockchain
            let pending = self.pending_trie_writes.read().map_err(|_| {
                StoreError::Custom("Failed to acquire read lock on pending trie writes".into())
            })?;

            Ok(Box::new(EthrexDbLockedView {
                blockchain: self.blockchain.clone(),
                table_name,
                pending_snapshot: pending.get(table_name).cloned().unwrap_or_default(),
            }))
        } else {
            self.auxiliary.begin_locked(table_name)
        }
    }

    fn create_checkpoint(&self, path: &Path) -> Result<(), StoreError> {
        // Create checkpoint for auxiliary RocksDB
        let aux_checkpoint_path = path.join("auxiliary");
        self.auxiliary.create_checkpoint(&aux_checkpoint_path)?;

        // Note: ethrex_db has its own snapshot mechanism via PagedDb::create_snapshot()
        // For now, we rely on finalization for state durability
        Ok(())
    }
}

/// Read view for the hybrid backend.
pub struct EthrexDbReadView<'a> {
    /// Reference to the blockchain for state reads.
    blockchain: Arc<RwLock<Blockchain>>,
    /// Auxiliary read view for non-trie data.
    auxiliary: Box<dyn StorageReadView + 'a>,
    /// Snapshot of pending trie writes at the time of view creation.
    pending_snapshot: HashMap<&'static str, HashMap<Vec<u8>, Vec<u8>>>,
}

impl<'a> StorageReadView for EthrexDbReadView<'a> {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        if is_ethrex_db_table(table) {
            // First check pending writes
            if let Some(table_data) = self.pending_snapshot.get(table) {
                if let Some(value) = table_data.get(key) {
                    return Ok(Some(value.clone()));
                }
            }

            // TODO: For full integration, we need to translate the nibble-based key
            // to ethrex_db's account/storage API. For now, return None for trie tables
            // as the actual integration will happen at a higher level (Store).
            //
            // The key format in ethrex is:
            // - Account trie: Nibbles of keccak256(address)
            // - Storage trie: address_prefix (17 separator) + Nibbles of keccak256(slot)
            //
            // ethrex_db uses:
            // - blockchain.get_finalized_account(&[u8; 20]) for accounts
            // - blockchain.get_finalized_storage_by_hash(&[u8; 32], &[u8; 32]) for storage
            Ok(None)
        } else {
            self.auxiliary.get(table, key)
        }
    }

    fn prefix_iterator(
        &self,
        table: &'static str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError> {
        if is_ethrex_db_table(table) {
            // Return iterator over pending writes matching prefix
            let results: Vec<PrefixResult> = self
                .pending_snapshot
                .get(table)
                .map(|table_data| {
                    table_data
                        .iter()
                        .filter(|(k, _)| k.starts_with(prefix))
                        .map(|(k, v)| {
                            Ok((k.clone().into_boxed_slice(), v.clone().into_boxed_slice()))
                        })
                        .collect()
                })
                .unwrap_or_default();

            Ok(Box::new(results.into_iter()))
        } else {
            self.auxiliary.prefix_iterator(table, prefix)
        }
    }
}

/// Write batch for the hybrid backend.
pub struct EthrexDbWriteBatch {
    /// Reference to the shared pending trie writes.
    pending_trie_writes: Arc<RwLock<HashMap<&'static str, HashMap<Vec<u8>, Vec<u8>>>>>,
    /// Local batch of trie writes to be merged on commit.
    trie_batch: HashMap<&'static str, Vec<(Vec<u8>, Vec<u8>)>>,
    /// Auxiliary write batch for non-trie data.
    auxiliary: Box<dyn StorageWriteBatch + 'static>,
}

impl StorageWriteBatch for EthrexDbWriteBatch {
    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        if is_ethrex_db_table(table) {
            self.trie_batch
                .entry(table)
                .or_default()
                .push((key.to_vec(), value.to_vec()));
            Ok(())
        } else {
            self.auxiliary.put(table, key, value)
        }
    }

    fn put_batch(
        &mut self,
        table: &'static str,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        if is_ethrex_db_table(table) {
            self.trie_batch.entry(table).or_default().extend(batch);
            Ok(())
        } else {
            self.auxiliary.put_batch(table, batch)
        }
    }

    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        if is_ethrex_db_table(table) {
            // For deletes, we could store a tombstone or handle differently
            // For now, just remove from pending writes if present
            let mut pending = self.pending_trie_writes.write().map_err(|_| {
                StoreError::Custom("Failed to acquire write lock on pending trie writes".into())
            })?;
            if let Some(table_data) = pending.get_mut(table) {
                table_data.remove(key);
            }
            Ok(())
        } else {
            self.auxiliary.delete(table, key)
        }
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        // Commit auxiliary writes to RocksDB
        self.auxiliary.commit()?;

        // Merge trie batch into pending writes
        if !self.trie_batch.is_empty() {
            let mut pending = self.pending_trie_writes.write().map_err(|_| {
                StoreError::Custom("Failed to acquire write lock on pending trie writes".into())
            })?;

            for (table, entries) in self.trie_batch.drain() {
                let table_data = pending.entry(table).or_default();
                for (key, value) in entries {
                    table_data.insert(key, value);
                }
            }
        }

        Ok(())
    }
}

/// Locked view for trie tables in the hybrid backend.
pub struct EthrexDbLockedView {
    /// Reference to the blockchain for state reads.
    blockchain: Arc<RwLock<Blockchain>>,
    /// The table this view is locked to.
    table_name: &'static str,
    /// Snapshot of pending writes for this table.
    pending_snapshot: HashMap<Vec<u8>, Vec<u8>>,
}

impl StorageLockedView for EthrexDbLockedView {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        // First check pending writes
        if let Some(value) = self.pending_snapshot.get(key) {
            return Ok(Some(value.clone()));
        }

        // TODO: Translate to ethrex_db API when full integration is complete
        // For now, return None as actual trie data access will happen at Store level
        let _ = self.blockchain; // Suppress unused warning
        let _ = self.table_name;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_hybrid_backend_creation() {
        let temp_dir = TempDir::new().unwrap();
        let backend = EthrexDbBackend::open(temp_dir.path()).unwrap();

        // Verify storage locations were created
        assert!(temp_dir.path().join("state.db").exists());
        assert!(temp_dir.path().join("auxiliary").exists());

        // Verify we can access both components
        let _blockchain = backend.blockchain();
        let _auxiliary = backend.auxiliary();
    }

    #[test]
    fn test_table_routing() {
        assert!(is_ethrex_db_table(ACCOUNT_TRIE_NODES));
        assert!(is_ethrex_db_table(STORAGE_TRIE_NODES));
        assert!(is_ethrex_db_table(ACCOUNT_FLATKEYVALUE));
        assert!(is_ethrex_db_table(STORAGE_FLATKEYVALUE));
        assert!(!is_ethrex_db_table("headers"));
        assert!(!is_ethrex_db_table("bodies"));
    }

    #[test]
    fn test_write_and_read_auxiliary() {
        let temp_dir = TempDir::new().unwrap();
        let backend = EthrexDbBackend::open(temp_dir.path()).unwrap();

        // Write to auxiliary table (headers)
        {
            let mut tx = backend.begin_write().unwrap();
            tx.put("headers", b"key1", b"value1").unwrap();
            tx.commit().unwrap();
        }

        // Read back
        {
            let tx = backend.begin_read().unwrap();
            let value = tx.get("headers", b"key1").unwrap();
            assert_eq!(value, Some(b"value1".to_vec()));
        }
    }

    #[test]
    fn test_write_and_read_trie_pending() {
        let temp_dir = TempDir::new().unwrap();
        let backend = EthrexDbBackend::open(temp_dir.path()).unwrap();

        // Write to trie table (goes to pending writes)
        {
            let mut tx = backend.begin_write().unwrap();
            tx.put(ACCOUNT_TRIE_NODES, b"trie_key", b"trie_value")
                .unwrap();
            tx.commit().unwrap();
        }

        // Read back from pending
        {
            let tx = backend.begin_read().unwrap();
            let value = tx.get(ACCOUNT_TRIE_NODES, b"trie_key").unwrap();
            assert_eq!(value, Some(b"trie_value".to_vec()));
        }
    }
}
