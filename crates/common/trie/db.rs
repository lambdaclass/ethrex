use crate::{NodeHash, error::TrieError};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
};

pub trait TrieDB: Send + Sync + TrieDbReader {
    fn read_tx<'a>(&'a self) -> Box<dyn 'a + TrieDbReader>;
}

pub trait TrieDbReader {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError>;
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
    fn read_tx<'a>(&'a self) -> Box<dyn 'a + TrieDbReader> {
        struct InnerReader<'a>(MutexGuard<'a, HashMap<NodeHash, Vec<u8>>>);

        impl TrieDbReader for InnerReader<'_> {
            fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
                Ok(self.0.get(&key).cloned())
            }
        }

        Box::new(InnerReader(self.inner.lock().expect("poisoned mutex")))
    }

    // fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
    //     let mut db = self.inner.lock().map_err(|_| TrieError::LockError)?;

    //     for (key, value) in key_values {
    //         db.insert(key, value);
    //     }

    //     Ok(())
    // }
}

impl TrieDbReader for InMemoryTrieDB {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let tx = self.read_tx();
        tx.get(key)
    }
}
