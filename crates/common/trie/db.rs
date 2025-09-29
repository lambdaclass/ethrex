use ethereum_types::H256;
use ethrex_rlp::encode::RLPEncode;

use crate::{Node, NodeHash, NodeRLP, Trie, error::TrieError};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub trait TrieDB: Send + Sync {
    fn get(&self, key: NodeHash) -> Result<Option<Vec<u8>>, TrieError>;
    fn put_batch(&self, key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError>;
    fn put_batch_no_alloc(&self, key_values: &Vec<Node>) -> Result<(), TrieError> {
        self.put_batch(
            key_values
                .into_iter()
                .map(|node| (node.compute_hash(), node.encode_to_vec()))
                .collect(),
        )
    }
    fn put(&self, key: NodeHash, value: Vec<u8>) -> Result<(), TrieError> {
        self.put_batch(vec![(key, value)])
    }
}

/// InMemory implementation for the TrieDB trait, with get and put operations.
#[derive(Default)]
pub struct InMemoryTrieDB {
    pub inner: Arc<Mutex<BTreeMap<NodeHash, Vec<u8>>>>,
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

    pub fn from_nodes(
        root_hash: H256,
        state_nodes: &BTreeMap<H256, NodeRLP>,
    ) -> Result<Self, TrieError> {
        let mut embedded_root = Trie::get_embedded_root(state_nodes, root_hash)?;
        let mut hashed_nodes: Vec<(NodeHash, Vec<u8>)> = vec![];
        embedded_root.commit(&mut hashed_nodes);

        let hashed_nodes = hashed_nodes.into_iter().collect();

        let in_memory_trie = Arc::new(Mutex::new(hashed_nodes));
        Ok(Self::new(in_memory_trie))
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
