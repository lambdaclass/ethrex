use crate::{NodeHash, error::TrieError};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[async_trait::async_trait]
pub trait TrieDB: Send + Sync {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError>;
    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError>;
    fn put(&self, key: NodeHash, value: Vec<u8>) -> Result<(), TrieError> {
        self.put_batch(vec![(key, value)])
    }
    async fn put_batch_async(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        self.put_batch(key_values)
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
        Ok(self
            .inner
            .lock()
            .map_err(|_| TrieError::LockError)?
            .get(&key)
            .cloned())
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let mut db = self.inner.lock().map_err(|_| TrieError::LockError)?;

        for (key, value) in key_values {
            db.insert(key, value);
        }

        Ok(())
    }
}
