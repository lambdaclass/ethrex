use crate::api::tables::{FLATKEY_VALUES, TRIE_NODES};
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
}

impl TrieDB for BackendTrieDB {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        let key = apply_prefix(self.address_prefix, key);
        self.last_computed_flatkeyvalue >= key
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let table = if key.is_leaf() {
            FLATKEY_VALUES
        } else {
            TRIE_NODES
        };
        let key = self.make_key(key);
        let tx = self.tx.lock().map_err(|_| TrieError::LockError)?;
        tx.get(table, &key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let mut tx = self.tx.lock().map_err(|_| TrieError::LockError)?;
        for (key, value) in key_values {
            let table = if key.is_leaf() {
                FLATKEY_VALUES
            } else {
                TRIE_NODES
            };
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
    trie_tx: Box<dyn StorageLocked>,
    fkv_tx: Box<dyn StorageLocked>,
    /// Last flatkeyvalue path already generated
    last_computed_flatkeyvalue: Nibbles,
    address_prefix: Option<H256>,
}

impl BackendTrieDBLocked {
    pub fn new(
        engine: &dyn StorageBackend,
        address_prefix: Option<H256>,
        last_written: Vec<u8>,
    ) -> Result<Self, StoreError> {
        let last_computed_flatkeyvalue = Nibbles::from_hex(last_written);
        let trie_tx = engine.begin_locked(TRIE_NODES)?;
        let fkv_tx = engine.begin_locked(FLATKEY_VALUES)?;
        Ok(Self {
            trie_tx,
            fkv_tx,
            last_computed_flatkeyvalue,
            address_prefix,
        })
    }
    fn make_key(&self, node_hash: Nibbles) -> Vec<u8> {
        apply_prefix(self.address_prefix, node_hash)
            .as_ref()
            .to_vec()
    }
}

impl TrieDB for BackendTrieDBLocked {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        let key = apply_prefix(self.address_prefix, key);
        self.last_computed_flatkeyvalue >= key
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let tx = if key.is_leaf() {
            &self.fkv_tx
        } else {
            &self.trie_tx
        };
        let key = self.make_key(key);
        tx.get(&key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        // Read-only locked storage, should not be used for puts
        Err(TrieError::DbError(anyhow::anyhow!("trie is read-only")))
    }
}
