use crate::api::tables::{
    ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, ACCOUNT_TRIE_TOP_NODES, STORAGE_FLATKEYVALUE,
    STORAGE_TRIE_NODES, STORAGE_TRIE_TOP_NODES,
};
use crate::api::{StorageBackend, StorageLockedView, StorageReadView};
use crate::error::StoreError;
use crate::layering::apply_prefix;
use ethrex_common::H256;
use ethrex_trie::{Nibbles, TrieDB, error::TrieError};
use std::sync::Arc;

/// TrieDB implementation that holds a pre-acquired read view for the entire
/// trie traversal, avoiding per-node-lookup allocation and lock acquisition.
/// Depth threshold for routing trie nodes to the dedicated top-node CFs.
/// Nodes with logical depth <= this value go to top-node CFs.
const TOP_NODE_DEPTH_THRESHOLD: usize = 10;

pub struct BackendTrieDB {
    /// Reference to the storage backend (used only for writes)
    db: Arc<dyn StorageBackend>,
    /// Pre-acquired read view held for the lifetime of this struct.
    /// Using Arc allows sharing a single read view across multiple BackendTrieDB
    /// instances (e.g., state trie + storage trie in a single query).
    read_view: Arc<dyn StorageReadView>,
    /// Last flatkeyvalue path already generated
    last_computed_flatkeyvalue: Nibbles,
    nodes_table: &'static str,
    top_nodes_table: &'static str,
    fkv_table: &'static str,
    /// Storage trie address prefix (for storage tries)
    /// None for state tries, Some(address) for storage tries
    address_prefix: Option<H256>,
}

impl BackendTrieDB {
    /// Create a new BackendTrieDB for the account trie
    pub fn new_for_accounts(
        db: Arc<dyn StorageBackend>,
        last_written: Vec<u8>,
    ) -> Result<Self, StoreError> {
        let read_view = db.begin_read()?;
        Self::new_for_accounts_with_view(db, read_view, last_written)
    }

    /// Create a new BackendTrieDB for the account trie with a shared read view
    pub fn new_for_accounts_with_view(
        db: Arc<dyn StorageBackend>,
        read_view: Arc<dyn StorageReadView>,
        last_written: Vec<u8>,
    ) -> Result<Self, StoreError> {
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);
        Ok(Self {
            db,
            read_view,
            last_computed_flatkeyvalue,
            nodes_table: ACCOUNT_TRIE_NODES,
            top_nodes_table: ACCOUNT_TRIE_TOP_NODES,
            fkv_table: ACCOUNT_FLATKEYVALUE,
            address_prefix: None,
        })
    }

    /// Create a new BackendTrieDB for the storage tries
    pub fn new_for_storages(
        db: Arc<dyn StorageBackend>,
        last_written: Vec<u8>,
    ) -> Result<Self, StoreError> {
        let read_view = db.begin_read()?;
        Self::new_for_storages_with_view(db, read_view, last_written)
    }

    /// Create a new BackendTrieDB for the storage tries with a shared read view
    pub fn new_for_storages_with_view(
        db: Arc<dyn StorageBackend>,
        read_view: Arc<dyn StorageReadView>,
        last_written: Vec<u8>,
    ) -> Result<Self, StoreError> {
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);
        Ok(Self {
            db,
            read_view,
            last_computed_flatkeyvalue,
            nodes_table: STORAGE_TRIE_NODES,
            top_nodes_table: STORAGE_TRIE_TOP_NODES,
            fkv_table: STORAGE_FLATKEYVALUE,
            address_prefix: None,
        })
    }

    /// Create a new BackendTrieDB for a specific storage trie
    pub fn new_for_account_storage(
        db: Arc<dyn StorageBackend>,
        address_prefix: H256,
        last_written: Vec<u8>,
    ) -> Result<Self, StoreError> {
        let read_view = db.begin_read()?;
        Self::new_for_account_storage_with_view(db, read_view, address_prefix, last_written)
    }

    /// Create a new BackendTrieDB for a specific storage trie with a shared read view
    pub fn new_for_account_storage_with_view(
        db: Arc<dyn StorageBackend>,
        read_view: Arc<dyn StorageReadView>,
        address_prefix: H256,
        last_written: Vec<u8>,
    ) -> Result<Self, StoreError> {
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);
        Ok(Self {
            db,
            read_view,
            last_computed_flatkeyvalue,
            nodes_table: STORAGE_TRIE_NODES,
            top_nodes_table: STORAGE_TRIE_TOP_NODES,
            fkv_table: STORAGE_FLATKEYVALUE,
            address_prefix: Some(address_prefix),
        })
    }

    fn make_key(&self, path: Nibbles) -> Vec<u8> {
        apply_prefix(self.address_prefix, path).into_vec()
    }

    /// Compute the logical depth of a key, accounting for the 65-nibble storage prefix.
    /// For account keys (no prefix): depth = key.len()
    /// For storage keys (65-nibble prefix): depth = key.len() - 65
    fn logical_trie_depth(&self, key: &[u8]) -> usize {
        if self.address_prefix.is_some() {
            key.len().saturating_sub(65)
        } else {
            key.len()
        }
    }

    /// Key might be for an account or storage slot.
    /// Returns (primary_table, is_top_node) where is_top_node indicates the key
    /// should be dual-written to both top-node and legacy CFs.
    fn table_for_key(&self, key: &[u8]) -> &'static str {
        let is_leaf = key.len() == 65 || key.len() == 131;
        if is_leaf {
            self.fkv_table
        } else if self.logical_trie_depth(key) <= TOP_NODE_DEPTH_THRESHOLD {
            self.top_nodes_table
        } else {
            self.nodes_table
        }
    }

    /// Returns true if this key is a top-depth trie node that should be dual-written.
    fn is_top_node(&self, key: &[u8]) -> bool {
        let is_leaf = key.len() == 65 || key.len() == 131;
        !is_leaf && self.logical_trie_depth(key) <= TOP_NODE_DEPTH_THRESHOLD
    }
}

impl TrieDB for BackendTrieDB {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        let key = apply_prefix(self.address_prefix, key);
        self.last_computed_flatkeyvalue >= key
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        use std::sync::atomic::Ordering::Relaxed;
        let prefixed_key = self.make_key(key);
        let table = self.table_for_key(&prefixed_key);
        let is_flat = table == self.fkv_table;
        if is_flat {
            crate::metrics::STORAGE_METRICS
                .flat_hits
                .fetch_add(1, Relaxed);
        } else {
            crate::metrics::STORAGE_METRICS
                .trie_node_reads
                .fetch_add(1, Relaxed);
        }
        let result = self
            .read_view
            .get(table, prefixed_key.as_ref())
            .map_err(|e| {
                TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e))
            })?;
        // Dual-read fallback: if the primary table (top-node CF) returned None,
        // fall back to the legacy nodes CF. This handles pre-existing data that
        // was written before CF isolation was enabled.
        if result.is_none() && self.is_top_node(&prefixed_key) {
            return self
                .read_view
                .get(self.nodes_table, prefixed_key.as_ref())
                .map_err(|e| {
                    TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e))
                });
        }
        Ok(result)
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let mut tx = self.db.begin_write().map_err(|e| {
            TrieError::DbError(anyhow::anyhow!("Failed to begin write transaction: {}", e))
        })?;
        for (key, value) in key_values {
            let prefixed_key = self.make_key(key);
            let table = self.table_for_key(&prefixed_key);
            // Dual-write: top-depth nodes go to both top-node CF and legacy CF.
            // This ensures rollback safety (reverting to legacy-only loses no data).
            if self.is_top_node(&prefixed_key) {
                tx.put_batch(self.nodes_table, vec![(prefixed_key.clone(), value.clone())])
                    .map_err(|e| {
                        TrieError::DbError(anyhow::anyhow!("Failed to write batch: {}", e))
                    })?;
            }
            tx.put_batch(table, vec![(prefixed_key, value)])
                .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to write batch: {}", e)))?;
        }
        tx.commit()
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to write batch: {}", e)))
    }
}

/// Read-only version with persistent locked transaction/snapshot for batch reads
pub struct BackendTrieDBLocked {
    account_trie_tx: Box<dyn StorageLockedView>,
    storage_trie_tx: Box<dyn StorageLockedView>,
    account_top_nodes_tx: Box<dyn StorageLockedView>,
    storage_top_nodes_tx: Box<dyn StorageLockedView>,
    account_fkv_tx: Box<dyn StorageLockedView>,
    storage_fkv_tx: Box<dyn StorageLockedView>,
    /// Last flatkeyvalue path already generated
    last_computed_flatkeyvalue: Nibbles,
}

impl BackendTrieDBLocked {
    pub fn new(engine: &dyn StorageBackend, last_written: Vec<u8>) -> Result<Self, StoreError> {
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);
        let account_trie_tx = engine.begin_locked(ACCOUNT_TRIE_NODES)?;
        let storage_trie_tx = engine.begin_locked(STORAGE_TRIE_NODES)?;
        let account_top_nodes_tx = engine.begin_locked(ACCOUNT_TRIE_TOP_NODES)?;
        let storage_top_nodes_tx = engine.begin_locked(STORAGE_TRIE_TOP_NODES)?;
        let account_fkv_tx = engine.begin_locked(ACCOUNT_FLATKEYVALUE)?;
        let storage_fkv_tx = engine.begin_locked(STORAGE_FLATKEYVALUE)?;
        Ok(Self {
            account_trie_tx,
            storage_trie_tx,
            account_top_nodes_tx,
            storage_top_nodes_tx,
            account_fkv_tx,
            storage_fkv_tx,
            last_computed_flatkeyvalue,
        })
    }

    /// Compute logical depth for a prefixed key.
    /// Account keys: len <= 65 → depth = len
    /// Storage keys: len > 65 → depth = len - 65
    fn logical_depth(key: &Nibbles) -> usize {
        if key.len() > 65 {
            key.len().saturating_sub(65)
        } else {
            key.len()
        }
    }

    /// Key is already prefixed. Returns the primary locked view for reads.
    fn tx_for_key(&self, key: &Nibbles) -> &dyn StorageLockedView {
        let is_leaf = key.len() == 65 || key.len() == 131;
        let is_account = key.len() <= 65;
        if is_leaf {
            if is_account {
                &*self.account_fkv_tx
            } else {
                &*self.storage_fkv_tx
            }
        } else if Self::logical_depth(key) <= TOP_NODE_DEPTH_THRESHOLD {
            // Short-path trie node → top-node CF
            if is_account {
                &*self.account_top_nodes_tx
            } else {
                &*self.storage_top_nodes_tx
            }
        } else if is_account {
            &*self.account_trie_tx
        } else {
            &*self.storage_trie_tx
        }
    }

    /// Fallback locked view for dual-read (legacy CF for top nodes).
    fn fallback_tx_for_key(&self, key: &Nibbles) -> Option<&dyn StorageLockedView> {
        let is_leaf = key.len() == 65 || key.len() == 131;
        if is_leaf {
            return None;
        }
        if Self::logical_depth(key) <= TOP_NODE_DEPTH_THRESHOLD {
            let is_account = key.len() <= 65;
            Some(if is_account {
                &*self.account_trie_tx
            } else {
                &*self.storage_trie_tx
            })
        } else {
            None
        }
    }
}

impl TrieDB for BackendTrieDBLocked {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        self.last_computed_flatkeyvalue >= key
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        use std::sync::atomic::Ordering::Relaxed;
        let is_leaf = key.len() == 65 || key.len() == 131;
        if is_leaf {
            crate::metrics::STORAGE_METRICS
                .flat_hits
                .fetch_add(1, Relaxed);
        } else {
            crate::metrics::STORAGE_METRICS
                .trie_node_reads
                .fetch_add(1, Relaxed);
        }
        let tx = self.tx_for_key(&key);
        let result = tx
            .get(key.as_ref())
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))?;
        // Dual-read fallback for top nodes
        if result.is_none()
            && let Some(fallback) = self.fallback_tx_for_key(&key)
        {
            return fallback.get(key.as_ref()).map_err(|e| {
                TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e))
            });
        }
        Ok(result)
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        // Read-only locked storage, should not be used for puts
        Err(TrieError::DbError(anyhow::anyhow!("trie is read-only")))
    }
}
