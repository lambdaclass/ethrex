use ethrex_common::H256;
use rustc_hash::FxHashMap;
use std::sync::Arc;

use ethrex_trie::{Nibbles, TrieDB, TrieError};

#[derive(Debug, Clone)]
pub(crate) struct TrieLayer {
    nodes: Arc<FxHashMap<Vec<u8>, Vec<u8>>>,
    parent: H256,
    id: usize,
    /// Per layer bloom filter, None if the size was exceeded (exceedingly rare).
    /// Having a bloom per layer avoids the cost of rehashing each key every time we rebuild the global bloom,
    /// since merge simply uses the u64 hashed keys instead of rehashing.
    bloom: Option<qfilter::Filter>,
}

#[derive(Clone, Debug)]
pub struct TrieLayerCache {
    /// Monotonically increasing ID for layers, starting at 1.
    /// TODO: this implementation panics on overflow
    last_id: usize,
    pub(crate) layers: FxHashMap<H256, Arc<TrieLayer>>,
    /// Global bloom that accrues all layer blooms.
    ///
    /// The bloom filter is used to avoid looking up all layers when the given path doesn't exist in any
    /// layer, thus going directly to the database.
    ///
    /// In case a bloom filter insert or merge fails, we need to mark the bloom filter as poisoned
    /// so we never use it again, because if we don't we may be misled into believing a key is not present
    /// on a diff layer when it is (i.e. a false negative), leading to wrong executions.
    pub(crate) bloom: Option<qfilter::Filter>,
}

impl Default for TrieLayerCache {
    fn default() -> Self {
        // Try to create the bloom filter, if it fails use poison mode.
        let bloom = Self::create_filter().ok();
        Self {
            bloom,
            last_id: 0,
            layers: Default::default(),
        }
    }
}

impl TrieLayerCache {
    // TODO: tune this
    fn create_filter() -> Result<qfilter::Filter, qfilter::Error> {
        qfilter::Filter::new_resizeable(1_000_000, 100_000_000, 0.02)
            .inspect_err(|e| tracing::warn!("could not create trie layering bloom filter {e}"))
    }

    pub fn get(&self, state_root: H256, key: Nibbles) -> Option<Vec<u8>> {
        let key = key.as_ref();

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

        let mut bloom = Self::create_filter().ok();

        // create the layer bloom, this is the only place where hashing of keys happens.
        if let Some(filter) = &mut bloom {
            for (p, _) in &key_values {
                if let Err(qfilter::Error::CapacityExceeded) = filter.insert(p.as_ref()) {
                    tracing::warn!("TrieLayerCache: put_batch per layer capacity exceeded");
                    bloom = None;
                    break;
                }
            }
        }

        // add this new bloom to the global one via merge
        if let Some(filter) = &mut self.bloom
            && let Some(new_filter) = &bloom
            && let Err(qfilter::Error::CapacityExceeded) = filter.merge(false, new_filter)
        {
            tracing::warn!("TrieLayerCache: put_batch merge capacity exceeded");
            self.bloom = None;
            bloom = None;
        }

        let nodes: FxHashMap<Vec<u8>, Vec<u8>> = key_values
            .into_iter()
            .map(|(path, value)| (path.into_vec(), value))
            .collect();

        self.last_id += 1;
        let entry = TrieLayer {
            nodes: Arc::new(nodes),
            parent,
            id: self.last_id,
            bloom,
        };
        self.layers.insert(state_root, Arc::new(entry));
    }

    /// Rebuilds the global bloom filter accruing all current existing layers.
    pub fn rebuild_bloom_threaded(
        layers: &FxHashMap<H256, Arc<TrieLayer>>,
    ) -> Option<qfilter::Filter> {
        let mut blooms = layers.values().map(|x| x.bloom.as_ref());

        let Some(mut ret) = blooms.next().flatten().cloned() else {
            tracing::warn!("TrieLayerCache: rebuild_bloom no valid bloom found");
            return None;
        };
        for bloom in blooms {
            let Some(bloom) = bloom else {
                tracing::warn!("TrieLayerCache: rebuild_bloom no valid bloom found");

                return None;
            };
            if let Err(qfilter::Error::CapacityExceeded) = ret.merge(false, bloom) {
                tracing::warn!("TrieLayerCache: rebuild_bloom capacity exceeded");

                return None;
            }
        }
        return Some(ret);
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
                .chain(layer.nodes.as_ref().clone())
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
        if let Some(value) = self.inner.get(self.state_root, key.clone()) {
            return Ok(Some(value));
        }
        self.db.get(key)
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        // TODO: Get rid of this.
        unimplemented!("This function should not be called");
    }
}
