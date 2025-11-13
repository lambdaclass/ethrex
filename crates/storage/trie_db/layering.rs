use ethrex_common::H256;
use rustc_hash::{FxBuildHasher, FxHashMap};
use std::sync::{Arc, Mutex};

use ethrex_trie::{Nibbles, TrieDB, TrieError};

#[derive(Debug, Clone)]
struct TrieLayer {
    nodes: FxHashMap<Vec<u8>, Vec<u8>>,
    parent: H256,
    id: usize,
}

#[derive(Clone, Debug)]
pub struct TrieLayerCache {
    /// Monotonically increasing ID for layers, starting at 1.
    /// TODO: this implementation panics on overflow
    last_id: usize,
    layers: FxHashMap<H256, Arc<TrieLayer>>,
    // node -> count, value
    accumulated_nodes: FxHashMap<Vec<u8>, (usize, Vec<u8>)>,
}

impl Default for TrieLayerCache {
    fn default() -> Self {
        Self {
            last_id: 0,
            layers: Default::default(),
            accumulated_nodes: Default::default(),
        }
    }
}

impl TrieLayerCache {
    pub fn get(&self, state_root: H256, key: &[u8]) -> Option<Vec<u8>> {
        if self.layers.contains_key(&state_root)
            && let Some((_count, value)) = self.accumulated_nodes.get(key)
        {
            return Some(value.clone());
        }

        None
    }

    // TODO: use finalized hash to know when to commit
    pub fn get_commitable(&self, mut state_root: H256, commit_threshold: usize) -> Option<H256> {
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
        if self.layers.contains_key(&state_root) {
            tracing::warn!("tried to insert a state_root that's already inserted");
            return;
        }

        let mut nodes = FxHashMap::with_capacity_and_hasher(key_values.len(), FxBuildHasher);
        for (path, value) in key_values
            .into_iter()
            .map(|(path, value)| (path.into_vec(), value))
        {
            self.accumulated_nodes
                .entry(path.clone())
                .and_modify(|x| {
                    x.0 += 1;
                    x.1 = value.clone()
                })
                .or_insert_with(|| (0, value.clone()));
            nodes.insert(path, value);
        }

        self.last_id += 1;
        let entry = TrieLayer {
            nodes,
            parent,
            id: self.last_id,
        };
        self.layers.insert(state_root, Arc::new(entry));
    }

    pub fn commit(&mut self, state_root: H256) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
        let layer = match Arc::try_unwrap(self.layers.remove(&state_root)?) {
            Ok(layer) => layer,
            Err(layer) => TrieLayer::clone(&layer),
        };
        // ensure parents are commited
        let parent_nodes = self.commit(layer.parent);
        // older layers are useless
        let removed = self.layers.extract_if(|_, item| item.id > layer.id);
        for (_, layer) in removed {
            for (key, _) in &layer.nodes {
                match self.accumulated_nodes.entry(key.clone()) {
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        let value = entry.get_mut();
                        value.0 -= 1;
                        if value.0 == 0 {
                            entry.remove();
                        }
                    }
                    _ => {}
                }
            }
        }

        Some(
            parent_nodes
                .unwrap_or_default()
                .into_iter()
                .chain(layer.nodes)
                .collect(),
        )
    }
}

pub struct TrieWrapper {
    pub state_root: H256,
    pub inner: Arc<Mutex<TrieLayerCache>>,
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
            .lock()
            .expect("poison")
            .get(self.state_root, key.as_ref())
        {
            return Ok(Some(value));
        }
        self.db.get(key)
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        // TODO: Get rid of this.
        unimplemented!("This function should not be called");
    }
}
