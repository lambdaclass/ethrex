use ethereum_types::H256;

use crate::{error::TrieError, Nibbles, NodeKey, NodeRLP, Trie};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub trait TrieDB: Send + Sync {
    fn get(&self, key: NodeKey) -> Result<Option<Vec<u8>>, TrieError>;
    fn put_batch(&self, key_values: Vec<(NodeKey, Vec<u8>)>) -> Result<(), TrieError>;
    fn put(&self, key: NodeKey, value: Vec<u8>) -> Result<(), TrieError> {
        self.put_batch(vec![(key, value)])
    }
}

/// InMemory implementation for the TrieDB trait, with get and put operations.
#[derive(Default)]
pub struct InMemoryTrieDB {
    pub inner: Arc<Mutex<BTreeMap<[u8; 65], Vec<u8>>>>,
}

impl InMemoryTrieDB {
    pub const fn new(map: Arc<Mutex<BTreeMap<[u8; 65], Vec<u8>>>>) -> Self {
        Self { inner: map }
    }
    pub fn new_empty() -> Self {
        Self {
            inner: Default::default(),
        }
    }

    pub fn from_nodes(
        root_hash: H256,
        state_nodes: &BTreeMap<H256, NodeRLP>,
    ) -> Result<Self, TrieError> {
        let mut embedded_root = Trie::get_embedded_root(state_nodes, root_hash)?;
        let mut hashed_nodes = vec![];
        embedded_root.commit(Nibbles::default(), &mut hashed_nodes);

        let hashed_nodes = hashed_nodes
            .into_iter()
            .map(|(k, v)| (k.to_fixed_size(), v))
            .collect();

        let in_memory_trie = Arc::new(Mutex::new(hashed_nodes));
        Ok(Self::new(in_memory_trie))
    }
}

impl TrieDB for InMemoryTrieDB {
    fn get(&self, key: NodeKey) -> Result<Option<Vec<u8>>, TrieError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| TrieError::LockError)?
            .get(&key.to_fixed_size())
            .cloned())
    }

    fn put_batch(&self, key_values: Vec<(NodeKey, Vec<u8>)>) -> Result<(), TrieError> {
        let mut db = self.inner.lock().map_err(|_| TrieError::LockError)?;

        for (key, value) in key_values {
            db.insert(key.to_fixed_size(), value);
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
