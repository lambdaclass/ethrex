use ethereum_types::H256;
use ethrex_rlp::encode::RLPEncode;

use crate::{Nibbles, Node, Trie, error::TrieError};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

// Nibbles -> encoded node
pub type NodeMap = Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>;

pub trait TrieDB: Send + Sync {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError>;
    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError>;
    // TODO: replace putbatch with this function.
    fn put_batch_no_alloc(&self, key_values: &[(Nibbles, Node)]) -> Result<(), TrieError> {
        self.put_batch(
            key_values
                .iter()
                .map(|node| (node.0.clone(), node.1.encode_to_vec()))
                .collect(),
        )
    }
    fn put(&self, key: Nibbles, value: Vec<u8>) -> Result<(), TrieError> {
        self.put_batch(vec![(key, value)])
    }
    /// Commits any pending changes to the underlying storage
    /// For read-only or in-memory implementations, this is a no-op
    fn commit(&self) -> Result<(), TrieError> {
        Ok(())
    }

}

// TODO: we should replace this with BackendTrieDB
/// InMemory implementation for the TrieDB trait, with get and put operations.
#[derive(Default)]
pub struct InMemoryTrieDB {
    inner: NodeMap,
    prefix: Option<Nibbles>,
}

impl InMemoryTrieDB {
    pub const fn new(map: NodeMap) -> Self {
        Self {
            inner: map,
            prefix: None,
        }
    }

    pub const fn new_with_prefix(map: NodeMap, prefix: Nibbles) -> Self {
        Self {
            inner: map,
            prefix: Some(prefix),
        }
    }

    pub fn new_empty() -> Self {
        Self {
            inner: Default::default(),
            prefix: None,
        }
    }

    // Do not remove or make private as we use this in ethrex-replay
    pub fn from_nodes(
        root_hash: H256,
        state_nodes: &BTreeMap<H256, Node>,
    ) -> Result<Self, TrieError> {
        let mut embedded_root = Trie::get_embedded_root(state_nodes, root_hash)?;
        let mut hashed_nodes = vec![];
        embedded_root.commit(Nibbles::default(), &mut hashed_nodes);

        let hashed_nodes = hashed_nodes
            .into_iter()
            .map(|(k, v)| (k.into_vec(), v))
            .collect();

        let in_memory_trie = Arc::new(Mutex::new(hashed_nodes));
        Ok(Self::new(in_memory_trie))
    }

    fn apply_prefix(&self, path: Nibbles) -> Nibbles {
        match &self.prefix {
            Some(prefix) => prefix.concat(&path),
            None => path,
        }
    }

    // Do not remove or make private as we use this in ethrex-replay
    pub fn inner(&self) -> NodeMap {
        Arc::clone(&self.inner)
    }
}

impl TrieDB for InMemoryTrieDB {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| TrieError::LockError)?
            .get(self.apply_prefix(key).as_ref())
            .cloned())
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let mut db = self.inner.lock().map_err(|_| TrieError::LockError)?;

        for (key, value) in key_values {
            let prefixed_key = self.apply_prefix(key);
            db.insert(prefixed_key.into_vec(), value);
        }

        Ok(())
    }
}
