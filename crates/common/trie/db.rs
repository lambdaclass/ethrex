use crate::{Nibbles, NodeHash, PathRLP, error::TrieError};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub trait TrieDB: Send + Sync {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError>;
    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError>;
    fn put(&self, key: Nibbles, value: Vec<u8>) -> Result<(), TrieError> {
        self.put_batch(vec![(key, value)])
    }
}

/// InMemory implementation for the TrieDB trait, with get and put operations.
pub struct InMemoryTrieDB {
    inner: Arc<Mutex<BTreeMap<[u8; 33], Vec<u8>>>>,
}

impl InMemoryTrieDB {
    pub const fn new(map: Arc<Mutex<BTreeMap<[u8; 33], Vec<u8>>>>) -> Self {
        Self { inner: map }
    }
    pub fn new_empty() -> Self {
        Self {
            inner: Default::default(),
        }
    }
}

impl TrieDB for InMemoryTrieDB {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| TrieError::LockError)?
            .get(&nibbles_to_fixed_size(key))
            .cloned())
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let mut db = self.inner.lock().map_err(|_| TrieError::LockError)?;

        for (key, value) in key_values {
            db.insert(nibbles_to_fixed_size(key), value);
        }

        Ok(())
    }
}

pub fn nibbles_to_fixed_size(nibbles: Nibbles) -> [u8; 33] {
    let node_hash_ref = nibbles.to_bytes();
    let original_len = node_hash_ref.len();

    let mut buffer = [0u8; 33];

    // Encode the node as [original_len, node_hash...]
    buffer[32] = nibbles.len() as u8;
    buffer[..original_len].copy_from_slice(&node_hash_ref);
    buffer
}
