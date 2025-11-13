use ethrex_common::H256;
use rustc_hash::FxHashMap;
use std::sync::Arc;

use ethrex_trie::{Nibbles, TrieDB, TrieError};

#[derive(Debug, Clone)]
struct TrieLayer {
    nodes: FxHashMap<Vec<u8>, Vec<u8>>,
    layers_map: FxHashMap<Vec<u8>, H256>,
    parent: H256,
    id: usize,
}

#[derive(Clone, Debug)]
pub struct TrieLayerCache {
    /// Monotonically increasing ID for layers, starting at 1.
    /// TODO: this implementation panics on overflow
    last_id: usize,
    layers: FxHashMap<H256, Arc<TrieLayer>>,
}

impl Default for TrieLayerCache {
    fn default() -> Self {
        Self {
            last_id: 0,
            layers: Default::default(),
        }
    }
}

impl TrieLayerCache {
    pub fn get(&self, state_root: H256, key: &[u8]) -> Option<Vec<u8>> {
        // Check if value is present in a layer.
        self.layers.get(&state_root).and_then(|trie_layer| {
            trie_layer.layers_map.get(key).and_then(|layer_key| {
                self.layers
                    .get(layer_key)
                    .and_then(|trie_layer| trie_layer.nodes.get(key).cloned())
            })
        })
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

        let nodes: FxHashMap<Vec<u8>, Vec<u8>> = key_values
            .clone()
            .into_iter()
            .map(|(path, value)| (path.into_vec(), value))
            .collect();

        self.last_id += 1;

        let values = key_values
            .into_iter()
            .map(|(path, _)| (path.into_vec(), state_root));

        let previous = self.layers.get(&parent).map_or_else(
            || values.clone().collect(),
            |trie_layer| {
                let mut previous = trie_layer.layers_map.clone();
                previous.extend(values.clone());
                previous
            },
        );

        let entry = TrieLayer {
            nodes: nodes.clone(),
            layers_map: previous,
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
        self.layers.retain(|_, item| item.id > layer.id);
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
    pub inner: Arc<TrieLayerCache>,
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
        self.inner
            .get(self.state_root, key.as_ref())
            .map_or_else(|| self.db.get(key), |v| Ok(Some(v)))
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        // TODO: Get rid of this.
        unimplemented!("This function should not be called");
    }
}
