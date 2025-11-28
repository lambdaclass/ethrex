use crate::api::tables::{
    ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES,
};
use crate::api::{StorageBackend, StorageLocked, StorageRwTx};
use crate::error::StoreError;
use crate::layering::apply_prefix;
use ethrex_common::H256;
use ethrex_trie::{Nibbles, TrieDB, error::TrieError};
use std::sync::Mutex;

/// StorageRwTx implementation for the TrieDB trait
/// Wraps a transaction to allow multiple trie operations on the same transaction
pub struct BackendTrieDB {
    /// Read-write transaction wrapped in Mutex for interior mutability
    tx: Mutex<Box<dyn StorageRwTx + 'static>>,
    /// Last flatkeyvalue path already generated
    last_computed_flatkeyvalue: Nibbles,
    /// Storage trie address prefix (for storage tries)
    /// None for state tries, Some(address) for storage tries
    address_prefix: Option<H256>,
}

impl BackendTrieDB {
    pub fn new(
        tx: Box<dyn StorageRwTx + 'static>,
        address_prefix: Option<H256>,
        last_written: Vec<u8>,
    ) -> Result<Self, StoreError> {
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);
        Ok(Self {
            tx: Mutex::new(tx),
            last_computed_flatkeyvalue,
            address_prefix,
        })
    }

    fn make_key(&self, path: Nibbles) -> Vec<u8> {
        apply_prefix(self.address_prefix, path).as_ref().to_vec()
    }

    /// Key might be for an account or storage slot
    fn table_for_key(&self, key: &Nibbles) -> &'static str {
        if key.is_leaf() {
            if self.address_prefix.is_some() {
                STORAGE_FLATKEYVALUE
            } else {
                ACCOUNT_FLATKEYVALUE
            }
        } else if self.address_prefix.is_some() {
            STORAGE_TRIE_NODES
        } else {
            ACCOUNT_TRIE_NODES
        }
    }
}

impl TrieDB for BackendTrieDB {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        let key = apply_prefix(self.address_prefix, key);
        self.last_computed_flatkeyvalue >= key
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let table = self.table_for_key(&key);
        let key = self.make_key(key);
        let tx = self.tx.lock().map_err(|_| TrieError::LockError)?;
        tx.get(table, &key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let mut tx = self.tx.lock().map_err(|_| TrieError::LockError)?;
        for (key, value) in key_values {
            let table = self.table_for_key(&key);
            tx.put_batch(table, vec![(self.make_key(key), value)])
                .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to write batch: {}", e)))?;
        }
        tx.commit()
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to write batch: {}", e)))
    }

    fn commit(&self) -> Result<(), TrieError> {
        self.tx
            .lock()
            .map_err(|_| TrieError::LockError)?
            .commit()
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to commit transaction: {}", e)))
    }
}

/// Read-only version with persistent locked transaction/snapshot for batch reads
pub struct BackendTrieDBLocked {
    account_trie_tx: Box<dyn StorageLocked>,
    storage_trie_tx: Box<dyn StorageLocked>,
    account_fkv_tx: Box<dyn StorageLocked>,
    storage_fkv_tx: Box<dyn StorageLocked>,
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

    /// Key is already prefixed
    fn tx_for_key(&self, key: &Nibbles) -> &dyn StorageLocked {
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
