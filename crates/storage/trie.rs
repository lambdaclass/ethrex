use crate::api::{StorageLocked, StorageRwTx};
use crate::layering::apply_prefix;
use ethrex_common::H256;
use ethrex_trie::{Nibbles, TrieDB, error::TrieError};
use std::sync::Mutex;

/// StorageRwTx implementation for the TrieDB trait
/// Wraps a transaction to allow multiple trie operations on the same transaction
pub struct BackendTrieDB {
    /// Read-write transaction wrapped in Mutex for interior mutability
    tx: Mutex<Box<dyn StorageRwTx + 'static>>,
    /// Table name for storing trie nodes
    table_name: &'static str,
    /// Storage trie address prefix (for storage tries)
    /// None for state tries, Some(address) for storage tries
    address_prefix: Option<H256>,
}

impl BackendTrieDB {
    pub fn new(
        tx: Box<dyn StorageRwTx + 'static>,
        table_name: &'static str,
        address_prefix: Option<H256>,
    ) -> Self {
        Self {
            tx: Mutex::new(tx),
            table_name,
            address_prefix,
        }
    }

    fn make_key(&self, node_hash: Nibbles) -> Vec<u8> {
        apply_prefix(self.address_prefix, node_hash)
            .as_ref()
            .to_vec()
    }
}

impl TrieDB for BackendTrieDB {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let key = self.make_key(key);
        let tx = self.tx.lock().map_err(|_| TrieError::LockError)?;
        tx.get(self.table_name, &key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let mut batch = Vec::with_capacity(key_values.len());
        for (node_hash, value) in key_values {
            batch.push((self.table_name, self.make_key(node_hash), value));
        }

        let mut tx = self.tx.lock().map_err(|_| TrieError::LockError)?;
        tx.put_batch(batch)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to write batch: {}", e)))?;
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
    lock: Box<dyn StorageLocked>,
    address_prefix: Option<H256>,
}

impl BackendTrieDBLocked {
    pub fn new(lock: Box<dyn StorageLocked>, address_prefix: Option<H256>) -> Self {
        Self {
            lock,
            address_prefix,
        }
    }
    fn make_key(&self, node_hash: Nibbles) -> Vec<u8> {
        apply_prefix(self.address_prefix, node_hash)
            .as_ref()
            .to_vec()
    }
}

impl TrieDB for BackendTrieDBLocked {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let key = self.make_key(key);
        self.lock
            .get(&key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put(&self, _key: Nibbles, _value: Vec<u8>) -> Result<(), TrieError> {
        // Read-only locked storage, should not be used for puts
        Err(TrieError::DbError(anyhow::anyhow!(
            "Cannot put in read-only locked storage"
        )))
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        // Read-only locked storage, should not be used for puts
        Err(TrieError::DbError(anyhow::anyhow!(
            "Cannot put_batch in read-only locked storage"
        )))
    }
}
