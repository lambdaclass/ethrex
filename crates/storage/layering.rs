use ethrex_common::H256;
use rustc_hash::FxHashMap;
use std::sync::Arc;

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
    /// Number of layers after which we should commit to the database.
    commit_threshold: usize,
    layers: FxHashMap<H256, Arc<TrieLayer>>,
    /// Global bloom that accrues all layer blooms.
    ///
    /// The bloom filter is used to avoid looking up all layers when the given path doesn't exist in any
    /// layer, thus going directly to the database.
    ///
    /// In case a bloom filter insert or merge fails, we need to mark the bloom filter as poisoned
    /// so we never use it again, because if we don't we may be misled into believing a key is not present
    /// on a diff layer when it is (i.e. a false negative), leading to wrong executions.
    bloom: Option<qfilter::Filter>,
}

impl Default for TrieLayerCache {
    fn default() -> Self {
        // Try to create the bloom filter, if it fails use poison mode.
        let bloom = Self::create_filter().ok();
        Self {
            bloom,
            last_id: 0,
            layers: Default::default(),
            commit_threshold: 128,
        }
    }
}

impl TrieLayerCache {
    pub fn new(commit_threshold: usize) -> Self {
        let bloom = Self::create_filter().ok();
        Self {
            bloom,
            last_id: 0,
            layers: Default::default(),
            commit_threshold,
        }
    }

    // TODO: tune this
    fn create_filter() -> Result<qfilter::Filter, qfilter::Error> {
        qfilter::Filter::new_resizeable(1_000_000, 100_000_000, 0.02)
            .inspect_err(|e| tracing::warn!("could not create trie layering bloom filter {e}"))
    }

    pub fn get(&self, state_root: H256, key: &[u8]) -> Option<Vec<u8>> {
        // Fast check to know if any layer may contains the given key.
        // We can only be certain it doesn't exist, but if it returns true it may or not exist (false positive).
        if let Some(filter) = &self.bloom
            && !filter.contains(key)
        {
            // TrieWrapper goes to db when returning None.
            return None;
        }

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

    // TODO: use finalized hash to know when to commit
    pub fn get_commitable(&self, mut state_root: H256) -> Option<H256> {
        let mut counter = 0;
        while let Some(layer) = self.layers.get(&state_root) {
            state_root = layer.parent;
            counter += 1;
            if counter > self.commit_threshold {
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
            // L1 always changes the state root (system contracts run even on empty blocks), so
            // this should not happen there. L2 can legitimately keep the same root on empty blocks
            // because it has no system contract calls.
            tracing::trace!("parent == state_root but key_values not empty");
            return;
        }
        if self.layers.contains_key(&state_root) {
            tracing::warn!("tried to insert a state_root that's already inserted");
            return;
        }

        // add this new bloom to the global one.
        if let Some(filter) = &mut self.bloom {
            for (p, _) in &key_values {
                if let Err(qfilter::Error::CapacityExceeded) = filter.insert(p.as_ref()) {
                    tracing::warn!("TrieLayerCache: put_batch capacity exceeded");
                    self.bloom = None;
                    break;
                }
            }
        }

        let nodes: FxHashMap<Vec<u8>, Vec<u8>> = key_values
            .into_iter()
            .map(|(path, value)| (path.into_vec(), value))
            .collect();

        self.last_id += 1;
        let entry = TrieLayer {
            nodes,
            parent,
            id: self.last_id,
        };
        self.layers.insert(state_root, Arc::new(entry));
    }

    /// Rebuilds the global bloom filter by inserting all keys from all layers.
    pub fn rebuild_bloom(&mut self) {
        let Ok(mut new_global_filter) = Self::create_filter() else {
            tracing::warn!(
                "TrieLayerCache: rebuild_bloom could not create new filter. Poisoning bloom."
            );
            self.bloom = None;
            return;
        };

        for layer in self.layers.values() {
            for path in layer.nodes.keys() {
                if let Err(qfilter::Error::CapacityExceeded) = new_global_filter.insert(path) {
                    tracing::warn!(
                        "TrieLayerCache: rebuild_bloom capacity exceeded. Poisoning bloom."
                    );
                    self.bloom = None;
                    return;
                }
            }
        }

        self.bloom = Some(new_global_filter);
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
        self.rebuild_bloom(); // layers removed, rebuild global bloom filter.
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
    /// Pre-computed prefix nibbles for storage tries.
    /// For state tries this is None; for storage tries this is
    /// `Nibbles::from_bytes(address.as_bytes()).append_new(17)`.
    prefix_nibbles: Option<Nibbles>,
}

impl TrieWrapper {
    pub fn new(
        state_root: H256,
        inner: Arc<TrieLayerCache>,
        db: Box<dyn TrieDB>,
        prefix: Option<H256>,
    ) -> Self {
        let prefix_nibbles =
            prefix.map(|p| Nibbles::from_bytes(p.as_bytes()).append_new(17));
        Self {
            state_root,
            inner,
            db,
            prefix_nibbles,
        }
    }
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
        // NOTE: we apply the prefix here, since the underlying TrieDB should
        // always be for the state trie.
        let key = match &self.prefix_nibbles {
            Some(prefix) => prefix.concat(&key),
            None => key,
        };
        self.db.flatkeyvalue_computed(key)
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let key = match &self.prefix_nibbles {
            Some(prefix) => prefix.concat(&key),
            None => key,
        };
        if let Some(value) = self.inner.get(self.state_root, key.as_ref()) {
            return Ok(Some(value));
        }
        self.db.get(key)
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        // TODO: Get rid of this.
        unimplemented!("This function should not be called");
    }
}
