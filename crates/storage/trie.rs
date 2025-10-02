use crate::api::{StorageLocked, StorageRwTx};
use ethrex_common::H256;
use ethrex_trie::{NodeHash, TrieDB, error::TrieError};
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

    fn make_key(&self, node_hash: &NodeHash) -> Vec<u8> {
        match &self.address_prefix {
            Some(address) => {
                let mut key = Vec::with_capacity(64);
                key.extend_from_slice(address.as_bytes());
                key.extend_from_slice(node_hash.as_ref());
                key
            }
            None => node_hash.as_ref().to_vec(),
        }
    }
}

impl TrieDB for BackendTrieDB {
    fn get(&self, node_hash: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let key = self.make_key(&node_hash);
        let tx = self.tx.lock().map_err(|_| TrieError::LockError)?;
        tx.get(self.table_name, &key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let mut batch = Vec::with_capacity(key_values.len());
        for (node_hash, value) in key_values {
            batch.push((self.table_name, self.make_key(&node_hash), value));
        }

        let mut tx = self.tx.lock().map_err(|_| TrieError::LockError)?;
        tx.put_batch(batch)
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

    fn make_key(&self, node_hash: NodeHash) -> Vec<u8> {
        match &self.address_prefix {
            Some(address) => {
                let mut key = Vec::with_capacity(64);
                key.extend_from_slice(address.as_bytes());
                key.extend_from_slice(node_hash.as_ref());
                key
            }
            None => node_hash.as_ref().to_vec(),
        }
    }
}

impl TrieDB for BackendTrieDBLocked {
    fn get(&self, node_hash: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let key = self.make_key(node_hash);
        self.lock
            .get(&key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put(&self, _node_hash: NodeHash, _value: Vec<u8>) -> Result<(), TrieError> {
        // Read-only locked storage, should not be used for puts
        Err(TrieError::DbError(anyhow::anyhow!(
            "Cannot put in read-only locked storage"
        )))
    }

    fn put_batch(&self, _key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        // Read-only locked storage, should not be used for puts
        Err(TrieError::DbError(anyhow::anyhow!(
            "Cannot put_batch in read-only locked storage"
        )))
    }
}
