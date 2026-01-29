use ethrex_common::H256;
use fastbloom::AtomicBloomFilter;
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHasher};
use std::{
    collections::HashMap,
    fmt,
    hash::{BuildHasher, Hasher},
    sync::Arc,
};

use ethrex_trie::{Nibbles, TrieDB, TrieError};

const BLOOM_SIZE: usize = 1_000_000;
const FALSE_POSITIVE_RATE: f64 = 0.02;

/// Extracts a u64 hash directly from a nibble path.
///
/// Since nibble paths are derived from Keccak256 hashes, they already have
/// excellent distribution. We extract the LAST 8 bytes (16 nibbles)
/// directly as a u64, avoiding redundant hashing.
///
/// Using the last bytes is important for storage trie paths which have the
/// structure: `hash(address) + separator + hash(storage_key)`. Using first
/// bytes would cause all storage slots from the same contract to collide.
///
/// For short paths (internal trie nodes), we fall back to FxHash.
#[inline]
fn path_to_hash(path: &[u8]) -> u64 {
    let mut len = path.len();

    // Skip leaf flag (16) if present at the end
    if len > 0 && path[len - 1] == 16 {
        len -= 1;
    }

    if len >= 16 {
        // Last 16 nibbles = last 8 bytes of the path's hash portion
        // Each nibble is stored as a byte with value 0-15
        let mut bytes = [0u8; 8];
        let start = len - 16;
        for i in 0..8 {
            bytes[i] = (path[start + i * 2] << 4) | path[start + i * 2 + 1];
        }
        u64::from_le_bytes(bytes)
    } else {
        // Short paths (branch/extension nodes): use FxHash as fallback
        let mut hasher = FxHasher::default();
        hasher.write(path);
        hasher.finish()
    }
}

/// A hasher that extracts hash values directly from nibble paths.
///
/// This is safe because nibble paths are derived from Keccak256 hashes,
/// so they already have uniform distribution. We extract the first
/// 8 bytes (16 nibbles) as the primary hash value.
///
/// The hasher accumulates multiple writes by XORing with the existing hash,
/// since the Hasher trait's write() can be called multiple times.
#[derive(Default)]
struct NibblePathHasher {
    hash: u64,
    /// Track if this is the first write (the actual path data)
    has_data: bool,
}

impl Hasher for NibblePathHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        if !self.has_data {
            // First write is the actual path data - use our optimized extraction
            self.hash = path_to_hash(bytes);
            self.has_data = true;
        } else {
            // Subsequent writes (e.g., length): mix in using FxHash's algorithm
            // This handles the case where HashMap calls write multiple times
            for byte in bytes {
                self.hash = self.hash.rotate_left(5) ^ (*byte as u64);
            }
        }
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        // Mix in the length using FxHash-style mixing
        self.hash = self.hash.wrapping_mul(0x517cc1b727220a95) ^ (i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

/// BuildHasher for NibblePathHasher.
#[derive(Default, Clone)]
struct NibblePathBuildHasher;

impl BuildHasher for NibblePathBuildHasher {
    type Hasher = NibblePathHasher;

    #[inline]
    fn build_hasher(&self) -> Self::Hasher {
        NibblePathHasher::default()
    }
}

/// HashMap optimized for nibble path keys (which are already hash-derived).
type NibblePathHashMap<K, V> = HashMap<K, V, NibblePathBuildHasher>;

#[derive(Debug, Clone)]
struct TrieLayer {
    nodes: NibblePathHashMap<Vec<u8>, Vec<u8>>,
    parent: H256,
    id: usize,
}

#[derive(Clone)]
pub struct TrieLayerCache {
    /// Monotonically increasing ID for layers, starting at 1.
    /// TODO: this implementation panics on overflow
    last_id: usize,
    /// Number of layers after which we should commit to the database.
    commit_threshold: usize,
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
            .field("layers", &self.layers)
            .field("bloom", &"AtomicBloomFilter")
            .finish()
    }
}

impl Default for TrieLayerCache {
    fn default() -> Self {
        Self {
            bloom: Self::create_filter(BLOOM_SIZE),
            last_id: 0,
            layers: Default::default(),
            commit_threshold: 128,
        }
    }
}

impl TrieLayerCache {
    pub fn new(commit_threshold: usize) -> Self {
        Self {
            bloom: Self::create_filter(BLOOM_SIZE),
            last_id: 0,
            layers: Default::default(),
            commit_threshold,
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
        // Use contains_hash to skip redundant hashing - the key is already derived from Keccak256.
        if !self.bloom.contains_hash(path_to_hash(key)) {
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

        // Add keys to the global bloom filter
        // Use insert_hash to skip redundant hashing - paths are already derived from Keccak256.
        for (p, _) in &key_values {
            self.bloom.insert_hash(path_to_hash(p.as_ref()));
        }

        let nodes: NibblePathHashMap<Vec<u8>, Vec<u8>> = key_values
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
        // Pre-compute total keys for optimal filter sizing
        let total_keys: usize = self.layers.values().map(|layer| layer.nodes.len()).sum();

        let filter = Self::create_filter(total_keys.max(BLOOM_SIZE));

        // Parallel insertion - AtomicBloomFilter allows concurrent insert via &self
        // Use insert_hash to skip redundant hashing - paths are already derived from Keccak256.
        self.layers.par_iter().for_each(|(_, layer)| {
            for path in layer.nodes.keys() {
                filter.insert_hash(path_to_hash(path));
            }
        });

        self.bloom = filter;
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
