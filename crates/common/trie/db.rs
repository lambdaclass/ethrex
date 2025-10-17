use ethereum_types::H256;
use ethrex_rlp::encode::RLPEncode;

use crate::{Nibbles, Node, NodeRLP, Trie, error::TrieError};
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
}

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

    pub fn from_nodes(
        root_hash: H256,
        state_nodes: &BTreeMap<H256, NodeRLP>,
    ) -> Result<Self, TrieError> {
        let mut embedded_root = Trie::get_embedded_root(state_nodes, root_hash)?;
        let mut hashed_nodes = vec![];
        let mut buf = Vec::new();
        embedded_root.commit(Nibbles::default(), &mut hashed_nodes, &mut buf);

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

pub fn nibbles_to_fixed_size(nibbles: Nibbles) -> [u8; 33] {
    let node_hash_ref = nibbles.to_bytes();
    let original_len = node_hash_ref.len();

    let mut buffer = [0u8; 33];

    // Encode the node as [node_path..., original_len]
    buffer[32] = nibbles.len() as u8;
    buffer[..original_len].copy_from_slice(&node_hash_ref);
    buffer
}
