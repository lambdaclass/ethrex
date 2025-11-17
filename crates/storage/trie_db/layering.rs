use ethrex_common::H256;
use ethrex_trie::{Nibbles, TrieDB, TrieError};
use rustc_hash::FxHashMap;
use std::{
    mem::take,
    sync::{Arc, RwLock},
    time::Instant,
};

#[derive(Debug, Clone)]
struct TrieLayer {
    nodes: FxHashMap<Vec<u8>, Vec<u8>>,
    prebuilt_clone: Option<FxHashMap<Vec<u8>, H256>>,
    layers_map: FxHashMap<Vec<u8>, H256>,
    parent: H256,
    id: usize,
}

#[derive(Clone, Debug)]
pub struct TrieLayerCache {
    /// Monotonically increasing ID for layers, starting at 1.
    /// TODO: this implementation panics on overflow
    last_id: usize,
    layers: FxHashMap<H256, Arc<RwLock<TrieLayer>>>,
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
            trie_layer
                .read()
                .unwrap()
                .layers_map
                .get(key)
                .and_then(|layer_key| {
                    self.layers
                        .get(layer_key)
                        .and_then(|trie_layer| trie_layer.read().unwrap().nodes.get(key).cloned())
                })
        })
    }

    // TODO: use finalized hash to know when to commit
    pub fn get_commitable(&self, mut state_root: H256, commit_threshold: usize) -> Option<H256> {
        let mut counter = 0;
        while let Some(layer) = self.layers.get(&state_root) {
            state_root = layer.read().unwrap().parent;
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
        key_values: Vec<(Vec<u8>, Vec<u8>)>,
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
        let mut now = Instant::now();

        let nodes: FxHashMap<Vec<u8>, Vec<u8>> = key_values.clone().into_iter().collect();
        tracing::info!("put_batch 1: nodes creation - elapsed {:?}", now.elapsed());
        now = Instant::now();

        self.last_id += 1;

        let layers_map = match self.layers.get(&parent) {
            Some(trie_layer) => {
                let mut t = trie_layer.write().unwrap();
                let mut previous = take(&mut t.prebuilt_clone).unwrap();
                tracing::info!(
                    "put_batch 2: layers_map.clone - elapsed {:?}",
                    now.elapsed()
                );
                now = Instant::now();
                previous.extend(key_values.into_iter().map(|(path, _)| (path, state_root)));
                tracing::info!("put_batch 3: previous extend - elapsed {:?}", now.elapsed());
                previous
            }
            None => {
                let a = key_values
                    .into_iter()
                    .map(|(path, _)| (path, state_root))
                    .collect();
                tracing::info!("put_batch 4: new layer_map - elapsed {:?}", now.elapsed());
                a
            }
        };
        now = Instant::now();

        let entry = TrieLayer {
            nodes,
            prebuilt_clone: None,
            layers_map,
            parent,
            id: self.last_id,
        };
        self.layers.insert(state_root, Arc::new(RwLock::new(entry)));
        tracing::info!("put_batch 5: layer insert - elapsed {:?}", now.elapsed());
    }

    pub fn prebuild_clones(&mut self) {
        self.layers.iter_mut().for_each(|(_k, trie_layer)| {
            let mut mut_layer = trie_layer.write().unwrap();
            if mut_layer.prebuilt_clone.is_none() {
                mut_layer.prebuilt_clone = Some(mut_layer.layers_map.clone());
            }
        });
    }

    fn remove_old_refs(&mut self, state_roots: &Vec<H256>) {
        tracing::info!("Layers to remove:  {:?}", state_roots);
        tracing::info!("Layers to remove: Layers amount {}", self.layers.len());
        self.layers.iter().for_each(|(_k, trie_layer)| {
            let mut t = trie_layer.write().unwrap();
            tracing::info!("Layers to remove: Layer {}, size before: {}", t.id, t.layers_map.len());
            t.layers_map.retain(|_, b| !state_roots.contains(b));
            tracing::info!("Layers to remove: Layer {}, size after: {}", t.id, t.layers_map.len());
        })
    }

    pub fn commit(&mut self, state_root: H256) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
        //         let layer = match Arc::try_unwrap(self.layers.remove(&state_root)?) {
        //             Ok(layer) => layer.into_inner().unwrap(),
        //             Err(layer) => TrieLayer::clone(&layer.read().unwrap()),
        //         };
        //         // ensure parents are commited
        //         let parent_nodes = self.commit(layer.parent);
        //         // older layers are useless
        //         self.layers
        //             .retain(|_, item| item.read().unwrap().id > layer.id);
        //         self.remove_old_refs(&state_root);
        //         Some(
        //             parent_nodes
        //                 .unwrap_or_default()
        //                 .into_iter()
        //                 .chain(layer.nodes)
        //                 .collect(),
        //         )
        let mut layers_to_commit = vec![];
        let mut roots_to_delete = vec![];
        let mut current_state_root = state_root;
        tracing::info!("commit 1: Layers amount {}", self.layers.len());
        while let Some(layer) = self.layers.remove(&current_state_root) {
            tracing::info!("commit 2: removing {current_state_root}");
            let layer = match Arc::try_unwrap(layer) {
                Ok(layer) => layer.into_inner().unwrap(),
                Err(layer) => TrieLayer::clone(&layer.read().unwrap()),
            };
            roots_to_delete.push(current_state_root);
            current_state_root = layer.parent;
            layers_to_commit.push(layer);
        }
        tracing::info!("commit 3: Layers amount {}", self.layers.len());
        let top_layer_id = layers_to_commit.first()?.id;
        // older layers are useless
        tracing::info!("commit 4: Layers amount {}", self.layers.len());
        self.layers
            .retain(|_, item| item.read().unwrap().id > top_layer_id);
        tracing::info!("commit 5: Layers amount {}", self.layers.len());
        self.remove_old_refs(&roots_to_delete);
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
