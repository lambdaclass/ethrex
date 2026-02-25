use crate::api::tables::{
    ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES,
};
use crate::api::{StorageBackend, StorageLockedView, StorageReadView};
use crate::error::StoreError;
use crate::layering::apply_prefix;
use ethrex_common::H256;
use ethrex_trie::{Nibbles, TrieDB, error::TrieError};
use std::sync::Arc;

/// TrieDB implementation that holds a pre-acquired read view for the entire
/// trie traversal, avoiding per-node-lookup allocation and lock acquisition.
pub struct BackendTrieDB {
    /// Reference to the storage backend (used only for writes)
    db: Arc<dyn StorageBackend>,
    /// Pre-acquired read view held for the lifetime of this struct.
    /// Using Arc allows sharing a single read view across multiple BackendTrieDB
    /// instances (e.g., state trie + storage trie in a single query).
    read_view: Arc<dyn StorageReadView>,
    nodes_table: &'static str,
    fkv_table: &'static str,
    /// Storage trie address prefix (for storage tries)
    /// None for state tries, Some(address) for storage tries
    address_prefix: Option<H256>,
}

impl BackendTrieDB {
    /// Create a new BackendTrieDB for the account trie
    pub fn new_for_accounts(
        db: Arc<dyn StorageBackend>,
    ) -> Result<Self, StoreError> {
        let read_view = db.begin_read()?;
        Self::new_for_accounts_with_view(db, read_view)
    }

    /// Create a new BackendTrieDB for the account trie with a shared read view
    pub fn new_for_accounts_with_view(
        db: Arc<dyn StorageBackend>,
        read_view: Arc<dyn StorageReadView>,
    ) -> Result<Self, StoreError> {
        Ok(Self {
            db,
            read_view,
            nodes_table: ACCOUNT_TRIE_NODES,
            fkv_table: ACCOUNT_FLATKEYVALUE,
            address_prefix: None,
        })
    }

    /// Create a new BackendTrieDB for the storage tries
    pub fn new_for_storages(
        db: Arc<dyn StorageBackend>,
    ) -> Result<Self, StoreError> {
        let read_view = db.begin_read()?;
        Self::new_for_storages_with_view(db, read_view)
    }

    /// Create a new BackendTrieDB for the storage tries with a shared read view
    pub fn new_for_storages_with_view(
        db: Arc<dyn StorageBackend>,
        read_view: Arc<dyn StorageReadView>,
    ) -> Result<Self, StoreError> {
        Ok(Self {
            db,
            read_view,
            nodes_table: STORAGE_TRIE_NODES,
            fkv_table: STORAGE_FLATKEYVALUE,
            address_prefix: None,
        })
    }

    /// Create a new BackendTrieDB for a specific storage trie
    pub fn new_for_account_storage(
        db: Arc<dyn StorageBackend>,
        address_prefix: H256,
    ) -> Result<Self, StoreError> {
        let read_view = db.begin_read()?;
        Self::new_for_account_storage_with_view(
            db,
            read_view,
            address_prefix,
        )
    }

    /// Create a new BackendTrieDB for a specific storage trie with a shared read view
    pub fn new_for_account_storage_with_view(
        db: Arc<dyn StorageBackend>,
        read_view: Arc<dyn StorageReadView>,
        address_prefix: H256,
    ) -> Result<Self, StoreError> {
        Ok(Self {
            db,
            read_view,
            nodes_table: STORAGE_TRIE_NODES,
            fkv_table: STORAGE_FLATKEYVALUE,
            address_prefix: Some(address_prefix),
        })
    }

    fn make_key(&self, path: Nibbles) -> Vec<u8> {
        apply_prefix(self.address_prefix, path).into_vec()
    }

    /// Key might be for an account or storage slot
    fn table_for_key(&self, key: &[u8]) -> &'static str {
        let is_leaf = key.len() == 65 || key.len() == 131;
        if is_leaf {
            self.fkv_table
        } else {
            self.nodes_table
        }
    }
}

impl TrieDB for BackendTrieDB {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let prefixed_key = self.make_key(key);
        let table = self.table_for_key(&prefixed_key);
        self.read_view
            .get(table, prefixed_key.as_ref())
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let mut tx = self.db.begin_write().map_err(|e| {
            TrieError::DbError(anyhow::anyhow!("Failed to begin write transaction: {}", e))
        })?;
        for (key, value) in key_values {
            let prefixed_key = self.make_key(key);
            let table = self.table_for_key(&prefixed_key);
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
    account_fkv_tx: Box<dyn StorageLockedView>,
    storage_fkv_tx: Box<dyn StorageLockedView>,
}

impl BackendTrieDBLocked {
    pub fn new(
        engine: &dyn StorageBackend,
    ) -> Result<Self, StoreError> {
        let account_trie_tx = engine.begin_locked(ACCOUNT_TRIE_NODES)?;
        let storage_trie_tx = engine.begin_locked(STORAGE_TRIE_NODES)?;
        let account_fkv_tx = engine.begin_locked(ACCOUNT_FLATKEYVALUE)?;
        let storage_fkv_tx = engine.begin_locked(STORAGE_FLATKEYVALUE)?;
        Ok(Self {
            account_trie_tx,
            storage_trie_tx,
            account_fkv_tx,
            storage_fkv_tx,
        })
    }

    /// Key is already prefixed
    fn tx_for_key(&self, key: &Nibbles) -> &dyn StorageLockedView {
        let is_leaf = key.len() == 65 || key.len() == 131;
        let is_account = key.len() <= 65;
        if is_leaf {
            if is_account {
                &*self.account_fkv_tx
            } else {
                &*self.storage_fkv_tx
            }
        } else if is_account {
            &*self.account_trie_tx
        } else {
            &*self.storage_trie_tx
        }
    }
}

impl TrieDB for BackendTrieDBLocked {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let tx = self.tx_for_key(&key);
        tx.get(key.as_ref())
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        // Read-only locked storage, should not be used for puts
        Err(TrieError::DbError(anyhow::anyhow!("trie is read-only")))
    }
}
