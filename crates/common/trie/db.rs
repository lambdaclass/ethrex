use crate::error::TrieError;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub trait TrieDB: Send + Sync {
    fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, TrieError>;
    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), TrieError>;
    // fn put_batch(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), TrieError>;
    fn put_batch(&self, key_values: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), TrieError>;
}

/// InMemory implementation for the TrieDB trait, with get and put operations.
pub struct InMemoryTrieDB {
    inner: Arc<Mutex<HashMap<Vec<u8>, Vec<u8>>>>,
}

impl InMemoryTrieDB {
    pub fn new(map: Arc<Mutex<HashMap<Vec<u8>, Vec<u8>>>>) -> Self {
        Self { inner: map }
    }
    pub fn new_empty() -> Self {
        Self {
            inner: Default::default(),
        }
    }
}

impl TrieDB for InMemoryTrieDB {
    fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, TrieError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| TrieError::LockError)?
            .get(&key)
            .cloned())
    }

    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), TrieError> {
        self.inner
            .lock()
            .map_err(|_| TrieError::LockError)?
            .insert(key, value);
        Ok(())
    }

    fn put_batch(&self, key_values: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), TrieError> {
        let mut db = self.inner.lock().map_err(|_| TrieError::LockError)?;

        for (key, value) in key_values {
            db.insert(key, value);
        }

        Ok(())
    }
}
