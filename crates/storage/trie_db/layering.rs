use anyhow::anyhow;
use ethrex_common::H256;
use ethrex_rlp::decode::RLPDecode;
use std::{collections::HashMap, sync::Arc, sync::RwLock};

use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, TrieDB, TrieError};

#[derive(Debug)]
struct TrieLayer {
    nodes: HashMap<Vec<u8>, Vec<u8>>,
    parent: H256,
    id: usize,
}

#[derive(Debug, Default)]
pub struct TrieWrapperInner {
    counter: usize,
    layers: HashMap<H256, TrieLayer>,
}

impl TrieWrapperInner {
    pub fn get(&self, mut state_root: H256, key: Nibbles) -> Option<Vec<u8>> {
        while let Some(layer) = self.layers.get(&state_root) {
            if let Some(value) = layer.nodes.get(key.as_ref()) {
                return Some(value.clone());
            }
            state_root = layer.parent;
        }
        None
    }
    pub fn get_commitable(&mut self, mut state_root: H256) -> Option<H256> {
        let mut counter = 0;
        while let Some(layer) = self.layers.get(&state_root) {
            state_root = layer.parent;
            counter += 1;
            if counter > 10 {
                return Some(state_root);
            }
        }
        None
    }
    pub fn put_batch(
        &mut self,
        parent: H256,
        state_root: H256,
        key_values: Vec<(Nibbles, Vec<u8>)>,
    ) {
        self.layers
            .entry(state_root)
            .or_insert_with(|| {
                self.counter += 1;
                TrieLayer {
                    nodes: HashMap::new(),
                    parent,
                    id: self.counter,
                }
            })
            .nodes
            .extend(
                key_values
                    .into_iter()
                    .map(|(path, node)| (path.as_ref().to_vec(), node)),
            );
    }
    pub fn commit(&mut self, state_root: H256) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut layer = self.layers.remove(&state_root)?;
        // ensure parents are commited
        let parent_nodes = self.commit(layer.parent);
        // older layers are useless
        self.layers.retain(|_, item| item.id > layer.id);
        Some(
            parent_nodes
                .unwrap_or_default()
                .into_iter()
                .chain(layer.nodes.drain())
                .collect(),
        )
    }
}

pub struct TrieWrapper {
    pub state_root: H256,
    pub inner: Arc<RwLock<TrieWrapperInner>>,
    pub db: Box<dyn TrieDB>,
    pub prefix: Option<H256>,
}

pub fn apply_prefix(prefix: Option<H256>, path: Nibbles) -> Nibbles {
    match prefix {
        Some(prefix) => Nibbles::from_bytes(prefix.as_bytes())
            .append_new(17)
            .concat(path),
        None => path,
    }
}

impl TrieDB for TrieWrapper {
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let key = apply_prefix(self.prefix, key);
        if let Some(value) = self
            .inner
            .read()
            .map_err(|_| TrieError::LockError)?
            .get(self.state_root, key.clone())
        {
            return Ok(Some(value));
        }
        self.db.get(key)
    }
    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let last_pair = key_values.iter().rev().find(|(_path, rlp)| !rlp.is_empty());
        let new_state_root = match last_pair {
            Some((_, noderlp)) => {
                let root_node = Node::decode(&noderlp)?;
                root_node.compute_hash().finalize()
            }
            None => *EMPTY_TRIE_HASH,
        };
        let mut inner = self.inner.write().map_err(|_| TrieError::LockError)?;
        inner.put_batch(
            self.state_root,
            new_state_root,
            key_values
                .into_iter()
                .map(move |(path, node)| (apply_prefix(self.prefix, path), node))
                .collect(),
        );
        Ok(())
    }
}
