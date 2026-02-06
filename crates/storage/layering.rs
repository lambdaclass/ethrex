use bytes::Bytes;
use ethrex_common::H256;
#[cfg(feature = "metrics")]
use ethrex_metrics::storage::METRICS_STORAGE;
use fastbloom::AtomicBloomFilter;
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap};
use std::{fmt, sync::Arc};

use ethrex_trie::{Nibbles, TrieDB, TrieError};

const BLOOM_SIZE: usize = 1_000_000;
const FALSE_POSITIVE_RATE: f64 = 0.02;

#[derive(Debug, Clone)]
struct TrieLayer {
    nodes: FxHashMap<Vec<u8>, Bytes>,
    parent: H256,
    id: usize,
}

#[derive(Clone)]
pub struct TrieLayerCache {
    /// Monotonically increasing ID for layers, starting at 1.
    /// TODO: this implementation panics on overflow
    last_id: usize,
    /// Maximum number of layers before committing to disk (fallback threshold).
    commit_threshold: usize,
    /// Maximum estimated memory (bytes) before committing to disk.
    max_memory_bytes: usize,
    /// Running estimate of memory used by all layers' node data.
    estimated_memory: usize,
    layers: FxHashMap<H256, Arc<TrieLayer>>,
    /// Global bloom filter that tracks all keys across all layers.
    ///
    /// Used to avoid looking up all layers when the given path doesn't exist in any
    /// layer, thus going directly to the database.
    bloom: AtomicBloomFilter<FxBuildHasher>,
}

impl fmt::Debug for TrieLayerCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrieLayerCache")
            .field("last_id", &self.last_id)
            .field("commit_threshold", &self.commit_threshold)
            .field("max_memory_bytes", &self.max_memory_bytes)
            .field("estimated_memory", &self.estimated_memory)
            .field("layers", &self.layers)
            .field("bloom", &"AtomicBloomFilter")
            .finish()
    }
}

/// Default maximum memory for trie layer cache (512 MB).
const DEFAULT_MAX_CACHE_MEMORY: usize = 512 * 1024 * 1024;

impl Default for TrieLayerCache {
    fn default() -> Self {
        Self {
            bloom: Self::create_filter(BLOOM_SIZE),
            last_id: 0,
            layers: Default::default(),
            commit_threshold: 128,
            max_memory_bytes: DEFAULT_MAX_CACHE_MEMORY,
            estimated_memory: 0,
        }
    }
}

impl TrieLayerCache {
    pub fn new(commit_threshold: usize) -> Self {
        let max_memory_bytes = std::env::var("ETHREX_TRIE_CACHE_MAX_MEMORY_MB")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .map(|mb| mb * 1024 * 1024)
            .unwrap_or(DEFAULT_MAX_CACHE_MEMORY);
        Self {
            bloom: Self::create_filter(BLOOM_SIZE),
            last_id: 0,
            layers: Default::default(),
            commit_threshold,
            max_memory_bytes,
            estimated_memory: 0,
        }
    }

    fn create_filter(expected_items: usize) -> AtomicBloomFilter<FxBuildHasher> {
        AtomicBloomFilter::with_false_pos(FALSE_POSITIVE_RATE)
            .hasher(FxBuildHasher)
            .expected_items(expected_items.max(BLOOM_SIZE))
    }

    pub fn get(&self, state_root: H256, key: &[u8]) -> Option<Vec<u8>> {
        // Fast check to know if any layer may contain the given key.
        // We can only be certain it doesn't exist, but if it returns true it may or may not exist (false positive).
        if !self.bloom.contains(key) {
            #[cfg(feature = "metrics")]
            METRICS_STORAGE.inc_layer_cache_misses();
            // TrieWrapper goes to db when returning None.
            return None;
        }

        let mut current_state_root = state_root;

        while let Some(layer) = self.layers.get(&current_state_root) {
            if let Some(value) = layer.nodes.get(key) {
                #[cfg(feature = "metrics")]
                METRICS_STORAGE.inc_layer_cache_hits();
                return Some(value.to_vec());
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
        #[cfg(feature = "metrics")]
        METRICS_STORAGE.inc_layer_cache_misses();
        None
    }

    // TODO: use finalized hash to know when to commit
    pub fn get_commitable(&self, mut state_root: H256) -> Option<H256> {
        let mut counter = 0;
        while let Some(layer) = self.layers.get(&state_root) {
            state_root = layer.parent;
            counter += 1;
            if counter > self.commit_threshold || self.estimated_memory > self.max_memory_bytes {
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

        // Add keys to the global bloom filter
        for (p, _) in &key_values {
            self.bloom.insert(p.as_ref());
        }

        let nodes: FxHashMap<Vec<u8>, Bytes> = key_values
            .into_iter()
            .map(|(path, value)| (path.into_vec(), Bytes::from(value)))
            .collect();

        // Track estimated memory: key + value + FxHashMap entry overhead (~24 bytes)
        let layer_memory: usize = nodes.iter().map(|(k, v)| k.len() + v.len() + 24).sum();
        self.estimated_memory += layer_memory;

        self.last_id += 1;
        let entry = TrieLayer {
            nodes,
            parent,
            id: self.last_id,
        };
        self.layers.insert(state_root, Arc::new(entry));
        #[cfg(feature = "metrics")]
        METRICS_STORAGE.set_layer_cache_layers(self.layers.len() as i64);
    }

    /// Rebuilds the global bloom filter by inserting all keys from all layers.
    pub fn rebuild_bloom(&mut self) {
        // Pre-compute total keys for optimal filter sizing
        let total_keys: usize = self.layers.values().map(|layer| layer.nodes.len()).sum();

        let filter = Self::create_filter(total_keys.max(BLOOM_SIZE));

        // Parallel insertion - AtomicBloomFilter allows concurrent insert via &self
        self.layers.par_iter().for_each(|(_, layer)| {
            for path in layer.nodes.keys() {
                filter.insert(path);
            }
        });

        self.bloom = filter;
    }

    pub fn commit(&mut self, state_root: H256) -> Option<Vec<(Vec<u8>, Bytes)>> {
        let mut layers_to_commit = vec![];
        let mut current_state_root = state_root;
        while let Some(layer) = self.layers.remove(&current_state_root) {
            let layer = Arc::unwrap_or_clone(layer);
            current_state_root = layer.parent;
            layers_to_commit.push(layer);
        }
        let top_layer_id = layers_to_commit.first()?.id;
        // older layers are useless
        let mut retained_memory: usize = 0;
        self.layers.retain(|_, item| {
            let keep = item.id > top_layer_id;
            if keep {
                retained_memory += item
                    .nodes
                    .iter()
                    .map(|(k, v)| k.len() + v.len() + 24)
                    .sum::<usize>();
            }
            keep
        });
        self.estimated_memory = retained_memory;
        #[cfg(feature = "metrics")]
        METRICS_STORAGE.set_layer_cache_layers(self.layers.len() as i64);
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
        // NOTE: we apply the prefix here, since the underlying TrieDB should
        // always be for the state trie.
        let key = apply_prefix(self.prefix, key);
        self.db.flatkeyvalue_computed(key)
    }

    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let key = apply_prefix(self.prefix, key);
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
