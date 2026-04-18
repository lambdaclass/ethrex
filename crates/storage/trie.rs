use crate::api::tables::{
    ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES,
};
use crate::api::{StorageBackend, StorageLockedView, StorageReadView};
use crate::error::StoreError;
use crate::trie_key::TrieKey;
use ethrex_common::H256;
use ethrex_trie::{Nibbles, TrieDB, error::TrieError};
use std::sync::Arc;

/// TrieDB implementation that holds a pre-acquired read view for the entire
/// trie traversal, avoiding per-node-lookup allocation and lock acquisition.
///
/// Table routing (which RocksDB column family to read/write) is handled by
/// [`TrieKey::table()`], based on the nibble-path length after prefix application.
pub struct BackendTrieDB {
    /// Reference to the storage backend (used only for writes)
    db: Arc<dyn StorageBackend>,
    /// Pre-acquired read view held for the lifetime of this struct.
    read_view: Arc<dyn StorageReadView>,
    /// Last flatkeyvalue path already generated
    last_computed_flatkeyvalue: Nibbles,
    /// Storage trie address prefix (for storage tries).
    /// None for state tries, Some(keccak(address)) for storage tries.
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
            address_prefix: Some(address_prefix),
        })
    }

    /// Build a [`TrieKey`] by applying this DB's address prefix to a raw path.
    fn make_trie_key(&self, path: Nibbles) -> TrieKey {
        TrieKey::with_prefix(self.address_prefix, path)
    }
}

impl TrieDB for BackendTrieDB {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        let trie_key = self.make_trie_key(key);
        self.last_computed_flatkeyvalue >= *trie_key.nibbles()
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let trie_key = self.make_trie_key(key);
        let table = trie_key.table();
        self.read_view
            .get(table, trie_key.as_bytes())
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let mut tx = self.db.begin_write().map_err(|e| {
            TrieError::DbError(anyhow::anyhow!("Failed to begin write transaction: {}", e))
        })?;
        for (key, value) in key_values {
            let trie_key = self.make_trie_key(key);
            let table = trie_key.table();
            tx.put_batch(table, vec![(trie_key.into_vec(), value)])
                .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to write batch: {}", e)))?;
        }
        tx.commit()
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to write batch: {}", e)))
    }
}

/// Read-only version with persistent locked transaction/snapshot for batch reads.
///
/// Unlike [`BackendTrieDB`], this acquires per-CF locked snapshots at creation
/// time, so that all reads within a batch see a consistent view.
pub struct BackendTrieDBLocked {
    account_trie_tx: Box<dyn StorageLockedView>,
    storage_trie_tx: Box<dyn StorageLockedView>,
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
        let account_fkv_tx = engine.begin_locked(ACCOUNT_FLATKEYVALUE)?;
        let storage_fkv_tx = engine.begin_locked(STORAGE_FLATKEYVALUE)?;
        Ok(Self {
            account_trie_tx,
            storage_trie_tx,
            account_fkv_tx,
            storage_fkv_tx,
            last_computed_flatkeyvalue,
        })
    }

    /// Select the locked view for the column family that this key belongs to.
    fn tx_for_key(&self, key: &Nibbles) -> &dyn StorageLockedView {
        let trie_key = TrieKey::from_nibbles(key.clone());
        if trie_key.is_leaf() {
            if trie_key.is_account() {
                &*self.account_fkv_tx
            } else {
                &*self.storage_fkv_tx
            }
        } else if trie_key.is_account() {
            &*self.account_trie_tx
        } else {
            &*self.storage_trie_tx
        }
    }
}

impl TrieDB for BackendTrieDBLocked {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        self.last_computed_flatkeyvalue >= key
    }

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
