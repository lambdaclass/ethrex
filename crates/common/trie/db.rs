use crate::{NodeHash, error::TrieError};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub trait TrieDB: Send + Sync {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError>;
    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError>;
    fn put(&self, key: NodeHash, value: Vec<u8>) -> Result<(), TrieError> {
        self.put_batch(vec![(key, value)])
    }
}

/// InMemory implementation for the TrieDB trait, with get and put operations.
pub struct InMemoryTrieDB {
    inner: Arc<Mutex<HashMap<NodeHash, Vec<u8>>>>,
}

impl InMemoryTrieDB {
    pub const fn new(map: Arc<Mutex<HashMap<NodeHash, Vec<u8>>>>) -> Self {
        Self { inner: map }
    }
    pub fn new_empty() -> Self {
        Self {
            inner: Default::default(),
        }
    }
}

impl TrieDB for InMemoryTrieDB {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let lock = self.inner.lock().map_err(|_| TrieError::LockError)?;
        Ok(lock.get(&key).map(|full| {
            let mut v = full.clone();
            if v.len() >= 8 {
                v.truncate(v.len() - 8);
            }
            v
        }))
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let mut db = self.inner.lock().map_err(|_| TrieError::LockError)?;

        for (key, mut value) in key_values {
            // Add 8 extra bytes to store the block number
            value.extend_from_slice(&[0u8; 8]);
            db.insert(key, value);
        }

        Ok(())
    }
}
