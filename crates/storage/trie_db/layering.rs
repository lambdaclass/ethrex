use ethrex_common::H256;
use ethrex_rlp::decode::RLPDecode;
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::{Arc, RwLock},
};

use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, TrieDB, TrieError};

const N_HASHES: usize = 18;
const BLOOM_HASH_BITS: u64 = 12;
const BLOOM_SIZE: usize = 1 << BLOOM_HASH_BITS;

#[derive(Debug)]
struct TrieLayer {
    nodes: HashMap<Vec<u8>, Vec<u8>>,
    parent: H256,
    id: usize,
    bloom: [u64; BLOOM_SIZE],
}

#[derive(Debug, Default)]
pub struct TrieLayerCache {
    /// Monotonically increasing ID for layers, starting at 1.
    /// TODO: this implementation panics on overflow
    last_id: usize,
    layers: HashMap<H256, TrieLayer>,
}

impl TrieLayerCache {
    pub fn get(&self, state_root: H256, key: &[u8]) -> Option<Vec<u8>> {
        let mut current_state_root = state_root;
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let hash_init = hasher.finish();
        while let Some(layer) = self.layers.get(&current_state_root) {
            let mut missing = false;
            let mut hash = hash_init;
            for i in 0..N_HASHES {
                let idx: u8 = (hash & 0x3F) as u8;
                hash >>= 6;
                let filter_index = hash & ((1 << BLOOM_HASH_BITS) - 1);
                if layer.bloom[filter_index as usize] & (1 << idx) == 0 {
                    missing = true;
                    break;
                }
                hash ^= hash_init >> i;
            }
            if missing {
                current_state_root = layer.parent;
                continue;
            }
            if let Some(value) = layer.nodes.get(key) {
                return Some(value.clone());
            }
            current_state_root = layer.parent;
            if current_state_root == state_root {
                // TODO: check if this is possible in practice
                // This can't happen in L1, due to system contracts irreversibly modifying state
                // at each block.
                // On L2, if no transactions are included in a block, the state root remains the same,
                // but we handle that case in put_batch. It may happen, however, if someone modifies
                // state with a privileged tx and later reverts it (since it doesn't update nonce).
                panic!("State cycle found");
            }
        }
        None
    }

    // TODO: use finalized hash to know when to commit
    pub fn get_commitable(
        &mut self,
        mut state_root: H256,
        commit_threshold: usize,
    ) -> Option<H256> {
        let mut counter = 0;
        while let Some(layer) = self.layers.get(&state_root) {
            state_root = layer.parent;
            counter += 1;
            if counter > commit_threshold {
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
        if parent == state_root && key_values.is_empty() {
            return;
        } else if parent == state_root {
            tracing::error!("Inconsistent state: parent == state_root but key_values not empty");
            return;
        }
        let layer = self.layers.entry(state_root).or_insert_with(|| {
            self.last_id += 1;
            TrieLayer {
                nodes: HashMap::new(),
                parent,
                id: self.last_id,
                bloom: [0u64; BLOOM_SIZE],
            }
        });

        for (key, _) in &key_values {
            let mut hasher = DefaultHasher::new();
            key.hash(&mut hasher);
            let hash_init = hasher.finish();
            let mut hash = hash_init;
            for i in 0..N_HASHES {
                let idx: u8 = (hash & 0x3F) as u8;
                hash >>= 6;
                let filter_index = hash & ((1 << BLOOM_HASH_BITS) - 1);
                layer.bloom[filter_index as usize] |= 1 << idx;
                hash ^= hash_init >> i;
            }
        }
        layer.nodes.extend(
            key_values
                .into_iter()
                .map(|(path, node)| (path.into_vec(), node)),
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
    pub inner: Arc<RwLock<TrieLayerCache>>,
    pub db: Box<dyn TrieDB>,
    pub prefix: Option<H256>,
}

pub fn apply_prefix(prefix: Option<H256>, path: Nibbles) -> Nibbles {
    // Apply a prefix with an invalid nibble (17) as a separator, to
    // differentiate between a state trie value and a storage trie root.
    match prefix {
        Some(prefix) => Nibbles::from_bytes(prefix.as_bytes())
            .append_new(17)
            .concat(&path),
        None => path,
    }
}

impl TrieDB for TrieWrapper {
    fn flatkeyvalue_computed(&self, key: Nibbles) -> bool {
        let key = apply_prefix(self.prefix, key);
        self.db.flatkeyvalue_computed(key)
    }
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let key = apply_prefix(self.prefix, key);
        if let Some(value) = self
            .inner
            .read()
            .map_err(|_| TrieError::LockError)?
            .get(self.state_root, key.as_ref())
        {
            return Ok(Some(value));
        }
        self.db.get(key)
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        // TODO: this is unused, because we call `TrieLayerCache::put_batch` directly
        let last_pair = key_values.iter().rev().find(|(_path, rlp)| !rlp.is_empty());
        let new_state_root = match last_pair {
            Some((_, noderlp)) => {
                let root_node = Node::decode(noderlp)?;
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
