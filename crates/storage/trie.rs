use crate::api::{StorageBackend, StorageLocked};
use ethrex_common::H256;
use ethrex_trie::{NodeHash, TrieDB, error::TrieError};
use std::sync::Arc;

/// StorageBackend implementation for the TrieDB trait
/// Works with any database that implements StorageBackend
pub struct BackendTrieDB {
    /// Storage backend
    backend: Arc<dyn StorageBackend>,
    /// Table name for storing trie nodes
    table_name: String,
    /// Storage trie address prefix (for storage tries)
    /// None for state tries, Some(address) for storage tries
    address_prefix: Option<H256>,
}

impl BackendTrieDB {
    pub fn new(
        backend: Arc<dyn StorageBackend>,
        table_name: &str,
        address_prefix: Option<H256>,
    ) -> Self {
        Self {
            backend,
            table_name: table_name.to_string(),
            address_prefix,
        }
    }

    fn make_key(&self, node_hash: NodeHash) -> Vec<u8> {
        match &self.address_prefix {
            Some(address) => {
                // For storage tries, prefix with address
                let mut key = address.as_bytes().to_vec();
                key.extend_from_slice(node_hash.as_ref());
                key
            }
            None => {
                // For state tries, use node hash directly
                node_hash.as_ref().to_vec()
            }
        }
    }
}

impl TrieDB for BackendTrieDB {
    fn get(&self, node_hash: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let key = self.make_key(node_hash);
        let txn = self.backend.begin_read().map_err(|e| {
            TrieError::DbError(anyhow::anyhow!("Failed to begin read transaction: {}", e))
        })?;

        txn.get(&self.table_name, &key)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to get from database: {}", e)))
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let mut batch = Vec::with_capacity(key_values.len());
        for (node_hash, value) in key_values {
            batch.push((self.table_name.as_str(), self.make_key(node_hash), value));
        }

        let mut txn = self.backend.begin_write().map_err(|e| {
            TrieError::DbError(anyhow::anyhow!("Failed to begin write transaction: {}", e))
        })?;

        txn.put_batch(batch)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("Failed to write batch: {}", e)))?;

        txn.commit()
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
                // For storage tries, prefix with address
                let mut key = address.as_bytes().to_vec();
                key.extend_from_slice(node_hash.as_ref());
                key
            }
            // For state tries, use node hash directly
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
