use crate::{NodeHash, error::TrieError};
use smallvec::SmallVec;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub trait TrieDB: Send + Sync {
    fn get(
        &self,
        prefix_len: usize,
        full_path: SmallVec<[u8; 32]>,
        node_hash: NodeHash,
    ) -> Result<Option<Vec<u8>>, TrieError>;
    fn put_batch(
        &self,
        key_values: Vec<(usize, SmallVec<[u8; 32]>, NodeHash, Vec<u8>)>,
    ) -> Result<(), TrieError>;
    fn put(
        &self,
        prefix_len: usize,
        full_path: SmallVec<[u8; 32]>,
        node_hash: NodeHash,
        value: Vec<u8>,
    ) -> Result<(), TrieError> {
        self.put_batch(vec![(prefix_len, full_path, node_hash, value)])
    }
}

/// InMemory implementation for the TrieDB trait, with get and put operations.
pub struct InMemoryTrieDB {
    inner: Arc<Mutex<BTreeMap<NodeHash, Vec<u8>>>>,
}

impl InMemoryTrieDB {
    pub const fn new(map: Arc<Mutex<BTreeMap<NodeHash, Vec<u8>>>>) -> Self {
        Self { inner: map }
    }
    pub fn new_empty() -> Self {
        Self {
            inner: Default::default(),
        }
    }
}

impl TrieDB for InMemoryTrieDB {
    fn get(
        &self,
        _prefix_len: usize,
        _full_path: SmallVec<[u8; 32]>,
        node_hash: NodeHash,
    ) -> Result<Option<Vec<u8>>, TrieError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| TrieError::LockError)?
            .get(&node_hash)
            .cloned())
    }

    fn put_batch(
        &self,
        key_values: Vec<(usize, SmallVec<[u8; 32]>, NodeHash, Vec<u8>)>,
    ) -> Result<(), TrieError> {
        let mut db = self.inner.lock().map_err(|_| TrieError::LockError)?;

        for (_prefix_len, _full_path, node_hash, value) in key_values {
            db.insert(node_hash, value);
        }

        Ok(())
    }
}
