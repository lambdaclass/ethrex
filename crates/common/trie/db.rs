use ethrex_rlp::decode::RLPDecode;

use crate::{error::TrieError, Node, NodeHash};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

pub trait TrieDB: Send + Sync {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError>;
    fn put(&self, key: NodeHash, value: Vec<u8>) -> Result<(), TrieError>;
    // fn put_batch(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), TrieError>;
    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError>;
    fn witness(&self) -> HashSet<Vec<u8>>;
    fn record_witness(&mut self);
}

/// InMemory implementation for the TrieDB trait, with get and put operations.
pub struct InMemoryTrieDB {
    inner: Arc<Mutex<HashMap<NodeHash, Vec<u8>>>>,
    witness: Arc<Mutex<HashSet<Vec<u8>>>>,
    record_witness: bool,
}

impl InMemoryTrieDB {
    pub fn new(map: Arc<Mutex<HashMap<NodeHash, Vec<u8>>>>) -> Self {
        Self {
            inner: map,
            witness: Arc::new(Mutex::new(HashSet::new())),
            record_witness: false,
        }
    }
    pub fn new_empty() -> Self {
        Self {
            inner: Default::default(),
            witness: Default::default(),
            record_witness: false,
        }
    }
}

impl TrieDB for InMemoryTrieDB {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let value = self
            .inner
            .lock()
            .map_err(|_| TrieError::LockError)?
            .get(&key)
            .cloned();
        if !self.record_witness {
            return Ok(value);
        }
        if let Some(value) = value.as_ref() {
            if let Ok(decoded) = Node::decode(value) {
                let mut lock = self.witness.lock().map_err(|_| TrieError::LockError)?;
                lock.insert(decoded.encode_raw());
            }
        }
        Ok(value)
    }

    fn put(&self, key: NodeHash, value: Vec<u8>) -> Result<(), TrieError> {
        self.inner
            .lock()
            .map_err(|_| TrieError::LockError)?
            .insert(key, value);
        Ok(())
    }

    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        let mut db = self.inner.lock().map_err(|_| TrieError::LockError)?;

        for (key, value) in key_values {
            db.insert(key, value);
        }

        Ok(())
    }

    fn record_witness(&mut self) {
        self.record_witness = true;
    }

    fn witness(&self) -> HashSet<Vec<u8>> {
        let lock = self.witness.lock().unwrap();
        lock.clone()
    }
}
