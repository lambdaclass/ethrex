use ethrex_common::H256;
use ethrex_trie::{Nibbles, TrieDB, TrieError};
use rustc_hash::FxHashMap;
use std::sync::{Arc, Mutex};

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
    stacked_layers: Arc<Mutex<FxHashMap<H256, FxHashMap<Vec<u8>, Arc<TrieLayer>>>>>,
}

impl Default for TrieLayerCache {
    fn default() -> Self {
        Self {
            stacked_layers: Default::default(),
            last_id: 0,
            layers: Default::default(),
        }
    }
}

impl TrieLayerCache {
    fn get(&self, state_root: H256, key: &[u8]) -> Option<Vec<u8>> {
        match self.stacked_layers.lock().unwrap().get(&state_root) {
            Some(stack) => stack
                .get(key)
                .and_then(|layer| layer.nodes.get(key).cloned()),
            None => {
                let mut current_state_root = state_root;

                while let Some(layer) = self.layers.get(&current_state_root) {
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
        }
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
        key_values: FxHashMap<Vec<u8>, Vec<u8>>,
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

        self.last_id += 1;
        let entry = Arc::new(TrieLayer {
            nodes: key_values.clone(),
            parent,
            id: self.last_id,
        });

        let mut sl = self.stacked_layers.lock().unwrap();
        match sl.remove(&parent) {
            Some(mut map) => {
                map.extend(key_values.into_iter().map(|(key, _)| (key, entry.clone())));
                tracing::info!("New layer map size {}", map.len());
                sl.insert(state_root, map);
            }
            None => {
                let mut map = self.stack_from_layers(&parent);
                map.extend(key_values.into_iter().map(|(key, _)| (key, entry.clone())));
                sl.insert(state_root, map);
            }
        }
        self.layers.insert(state_root, entry.clone());
    }

    fn stack_from_layers(&self, state_root: &H256) -> FxHashMap<Vec<u8>, Arc<TrieLayer>> {
        let mut layers_to_stack = vec![];
        let mut current_state_root = state_root;
        while let Some(layer) = self.layers.get(current_state_root) {
            layers_to_stack.push(layer);
            current_state_root = &layer.parent;
        }
        let mut map: FxHashMap<Vec<u8>, Arc<TrieLayer>> = Default::default();
        map.extend(layers_to_stack.into_iter().rev().flat_map(|entry| {
            entry
                .nodes
                .iter()
                .map(|(key, _)| (key.clone(), entry.clone()))
        }));
        tracing::info!("Built stack from layers, map size: {}", map.len());
        map
    }

    pub fn commit(&mut self, state_root: H256) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut layers_to_commit = vec![];
        let mut current_state_root = state_root;
        while let Some(layer) = self.layers.remove(&current_state_root) {
            let layer = Arc::unwrap_or_clone(layer);
            current_state_root = layer.parent;
            layers_to_commit.push(layer);
        }
        let top_layer_id = layers_to_commit.first()?.id;
        // older layers are useless
        self.layers.retain(|_, item| item.id > top_layer_id);
        self.stacked_layers
            .lock()
            .unwrap()
            .iter_mut()
            .for_each(|(_, map)| map.retain(|_, item| item.id > top_layer_id));
        let nodes_to_commit = layers_to_commit
            .into_iter()
            .rev()
            .flat_map(|layer| layer.nodes)
            .collect();
        Some(nodes_to_commit)
    }
}

pub struct TrieWrapper {
    pub state_root: H256,
    pub inner: Arc<TrieLayerCache>,
    pub db: Box<dyn TrieDB>,
    pub prefix: Option<H256>,
}

pub fn apply_prefix(prefix: Option<H256>, path: Nibbles) -> Nibbles {
    match prefix {
        Some(prefix) => build_prefix(prefix).concat(&path),
        None => path,
    }
}

pub fn build_prefix(prefix: H256) -> Nibbles {
    // Apply a prefix with an invalid nibble (17) as a separator, to
    // differentiate between a state trie value and a storage trie root.
    Nibbles::from_bytes(prefix.as_bytes()).append_new(17)
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
