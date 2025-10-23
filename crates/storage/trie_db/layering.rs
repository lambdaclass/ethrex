use ethrex_common::H256;
use ethrex_rlp::decode::RLPDecode;
use std::{
    collections::HashMap,
    sync::{
        Arc, RwLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, TrieDB, TrieError};

#[derive(Debug)]
struct TrieLayer {
    nodes: HashMap<Vec<u8>, Vec<u8>>,
    parent: H256,
    id: usize,
}

#[derive(Debug, Default)]
pub struct TrieLayerCache {
    /// Monotonically increasing ID for layers, starting at 1.
    /// TODO: this implementation panics on overflow
    last_id: usize,
    layers: HashMap<H256, TrieLayer>,

    // Layer cache stats: `(hit_count, miss_count)`.
    stats: (AtomicUsize, AtomicUsize, AtomicUsize, bool),
}

impl TrieLayerCache {
    pub fn get(&self, state_root: H256, key: Nibbles) -> Option<Vec<u8>> {
        let mut current_state_root = state_root;
        // TODO: Why a `HashMap` and not a vec with a relative depth direct index?
        let mut index = 0;
        while let Some(layer) = self.layers.get(&current_state_root) {
            if let Some(value) = layer.nodes.get(key.as_ref()) {
                self.stats.0.fetch_add(1, Ordering::Relaxed);
                self.stats.2.fetch_add(index, Ordering::Relaxed);
                return Some(value.clone());
            }

            current_state_root = layer.parent;
            assert_ne!(current_state_root, state_root, "state cycle found");

            index += 1;
        }

        self.stats.1.fetch_add(1, Ordering::Relaxed);
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
        self.layers
            .entry(state_root)
            .or_insert_with(|| {
                self.last_id += 1;
                TrieLayer {
                    nodes: HashMap::new(),
                    parent,
                    id: self.last_id,
                }
            })
            .nodes
            .extend(
                key_values
                    .into_iter()
                    .map(|(path, node)| (path.into_vec(), node)),
            );
    }

    pub fn commit(&mut self, state_root: H256) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
        if !self.stats.3 {
            self.stats.3 = true;

            println!("#### Final stats:");
            let n_hits = self.stats.0.swap(0, Ordering::Relaxed);
            println!(
                "####   Hit  count: {n_hits} (at average layer depth of {})",
                self.stats.2.swap(0, Ordering::Relaxed) as f64 / n_hits as f64,
            );
            println!(
                "####   Miss count: {}",
                self.stats.1.swap(0, Ordering::Relaxed),
            );
        }

        let mut layer = self.layers.remove(&state_root)?;
        // ensure parents are commited
        let parent_nodes = self.commit(layer.parent);
        // older layers are useless
        self.layers.retain(|_, item| item.id > layer.id);

        self.stats.3 = false;
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
            .get(self.state_root, key.clone())
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
