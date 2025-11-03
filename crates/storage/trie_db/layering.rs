use ethrex_common::H256;
use rayon::iter::{ParallelBridge, ParallelIterator};
use rayon::slice::ParallelSliceMut;
use rustc_hash::FxHashMap;
use std::hash::BuildHasher;
use std::sync::Arc;

use ethrex_trie::{Nibbles, TrieDB, TrieError};

#[derive(Debug, Clone)]
struct TrieLayer {
    nodes: Arc<FxHashMap<Vec<u8>, Vec<u8>>>,
    parent: H256,
    id: usize,
}

#[derive(Clone)]
pub struct TrieLayerCache {
    /// Monotonically increasing ID for layers, starting at 1.
    /// TODO: this implementation panics on overflow
    last_id: usize,
    layers: FxHashMap<H256, Arc<TrieLayer>>,
    /// Global bloom that accrues all layer blooms.
    ///
    /// The bloom filter is used to avoid looking up all layers when the given path doesn't exist in any
    /// layer, thus going directly to the database.
    ///
    /// In case a bloom filter insert or merge fails, we need to mark the bloom filter as poisoned
    /// so we never use it again, because if we don't we may be misled into believing a key is not present
    /// on a diff layer when it is (i.e. a false negative), leading to wrong executions.
    bloom: Option<Arc<xorfilter::Fuse8>>,
}

impl Default for TrieLayerCache {
    fn default() -> Self {
        // Try to create the bloom filter, if it fails use poison mode.
        Self {
            bloom: None,
            last_id: 0,
            layers: Default::default(),
        }
    }
}

impl std::fmt::Debug for TrieLayerCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrieLayerCache")
            .field("last_id", &self.last_id)
            .field("layers", &self.layers)
            // bloom doesn't implement Debug
            .finish_non_exhaustive()
    }
}

impl TrieLayerCache {
    // TODO: tune this
    fn create_filter() -> xorfilter::Fuse8 {
        xorfilter::Fuse8::new(1_000_000)
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

        let nodes: FxHashMap<Vec<u8>, Vec<u8>> = key_values
            .into_iter()
            .map(|(path, value)| (path.into_vec(), value))
            .collect();

        self.last_id += 1;
        let entry = TrieLayer {
            nodes: Arc::new(nodes),
            parent,
            id: self.last_id,
        };
        self.layers.insert(state_root, Arc::new(entry));
        // We need to rebuild the filter, with xorfilter we can't simply add the layer since it's static.
        self.rebuild_bloom();
    }

    /// Rebuilds the global bloom filter accruing all current existing layers.
    pub fn rebuild_bloom(&mut self) {
        let mut bloom = Self::create_filter();

        // Parallelize key hashing ourselves because populate from xorfilter doesn't.
        let mut key_hashes: Vec<u64> = self
            .layers
            .values()
            .flat_map(|x| x.nodes.keys())
            .par_bridge()
            .map(|key| bloom.hash_builder.hash_one(key))
            .collect();

        // xorfilter needs "few" or no unique keys, so we need to do this.
        key_hashes.par_sort_unstable();
        key_hashes.dedup();

        bloom.populate_keys(&key_hashes);

        if let Err(e) = bloom.build() {
            tracing::warn!("TrieLayerCache: rebuild_bloom error: {e}");
            self.bloom = None;
            return;
        }

        self.bloom = Some(Arc::new(bloom));
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
        self.rebuild_bloom(); // layers removed, rebuild global bloom filter.
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
