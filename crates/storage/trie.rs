use crate::api::tables::{
    ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES,
};
use crate::api::{StorageBackend, StorageLockedView, StorageWriteBatch};
use crate::error::StoreError;
use crate::layering::{apply_prefix, prefixed_to_compact_db_key, to_compact_db_key};
use ethrex_common::H256;
use ethrex_trie::{Nibbles, TrieDB, error::TrieError};
use std::sync::Arc;

/// StorageWriteBatch implementation for the TrieDB trait
/// Wraps a transaction to allow multiple trie operations on the same transaction
pub struct BackendTrieDB {
    /// Reference to the storage backend
    db: Arc<dyn StorageBackend>,
    /// Last flatkeyvalue path already generated
    last_computed_flatkeyvalue: Nibbles,
    nodes_table: &'static str,
    fkv_table: &'static str,
    /// Storage trie address prefix (for storage tries)
    /// None for state tries, Some(address) for storage tries
    address_prefix: Option<H256>,
    /// When true, writes bypass the write-ahead log for higher throughput.
    /// Use during snap sync where data can be re-downloaded on crash.
    no_wal: bool,
}

impl BackendTrieDB {
    /// Create a new BackendTrieDB for the account trie
    pub fn new_for_accounts(
        db: Arc<dyn StorageBackend>,
        last_written: Vec<u8>,
    ) -> Result<Self, StoreError> {
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);
        Ok(Self {
            db,
            last_computed_flatkeyvalue,
            nodes_table: ACCOUNT_TRIE_NODES,
            fkv_table: ACCOUNT_FLATKEYVALUE,
            address_prefix: None,
            no_wal: false,
        })
    }

    /// Create a new BackendTrieDB for the storage tries
    pub fn new_for_storages(
        db: Arc<dyn StorageBackend>,
        last_written: Vec<u8>,
    ) -> Result<Self, StoreError> {
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);
        Ok(Self {
            db,
            last_computed_flatkeyvalue,
            nodes_table: STORAGE_TRIE_NODES,
            fkv_table: STORAGE_FLATKEYVALUE,
            address_prefix: None,
            no_wal: false,
        })
    }

    /// Create a new BackendTrieDB for a specific storage trie
    pub fn new_for_account_storage(
        db: Arc<dyn StorageBackend>,
        address_prefix: H256,
        last_written: Vec<u8>,
    ) -> Result<Self, StoreError> {
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);
        Ok(Self {
            db,
            last_computed_flatkeyvalue,
            nodes_table: STORAGE_TRIE_NODES,
            fkv_table: STORAGE_FLATKEYVALUE,
            address_prefix: Some(address_prefix),
            no_wal: false,
        })
    }

    /// Enable no-WAL mode for snap sync writes.
    pub fn with_no_wal(mut self) -> Self {
        self.no_wal = true;
        self
    }

    fn make_key(&self, path: &Nibbles) -> Vec<u8> {
        match self.address_prefix {
            Some(_) => to_compact_db_key(self.address_prefix, path),
            None => prefixed_to_compact_db_key(path),
        }
    }
}

impl TrieDB for BackendTrieDB {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        let key = apply_prefix(self.address_prefix, key);
        self.last_computed_flatkeyvalue >= key
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let table = if key.is_leaf() {
            self.fkv_table
        } else {
            self.nodes_table
        };
        let compact_key = self.make_key(&key);
        let tx = self.db.begin_read().map_err(|e| {
            TrieError::DbError(anyhow::anyhow!("Failed to begin read transaction: {}", e))
        })?;
        tx.get(table, &compact_key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let mut tx = self.db.begin_write().map_err(|e| {
            TrieError::DbError(anyhow::anyhow!("Failed to begin write transaction: {}", e))
        })?;
        for (key, value) in key_values {
            let table = if key.is_leaf() {
                self.fkv_table
            } else {
                self.nodes_table
            };
            let compact_key = self.make_key(&key);
            tx.put_batch(table, vec![(compact_key, value)])
                .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to write batch: {}", e)))?;
        }
        let commit_fn = if self.no_wal {
            StorageWriteBatch::commit_no_wal
        } else {
            StorageWriteBatch::commit
        };
        commit_fn(&mut *tx)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to write batch: {}", e)))
    }
}

/// Read-only version with persistent locked transaction/snapshot for batch reads
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

    /// Key is already prefixed (raw nibbles from apply_prefix).
    /// Uses is_leaf() and length threshold to select the correct table transaction.
    fn tx_for_key(&self, key: &Nibbles) -> &dyn StorageLockedView {
        let is_leaf = key.is_leaf();
        // Storage keys have [64 nibbles][16][17][...], so len >= 66
        let is_account = key.len() < 66;
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
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        self.last_computed_flatkeyvalue >= key
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let tx = self.tx_for_key(&key);
        let compact = prefixed_to_compact_db_key(&key);
        tx.get(&compact)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        // Read-only locked storage, should not be used for puts
        Err(TrieError::DbError(anyhow::anyhow!("trie is read-only")))
    }
}
