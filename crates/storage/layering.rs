use ethrex_common::H256;
use ethrex_trie::{Nibbles, Node, TrieCommitEntry, TrieDB, TrieError};
use fastbloom::AtomicBloomFilter;
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap};
use std::{fmt, sync::Arc};

const BLOOM_SIZE: usize = 1_000_000;
const FALSE_POSITIVE_RATE: f64 = 0.02;

/// A cached trie node entry holding both the decoded node and its RLP encoding.
#[derive(Clone, Debug)]
pub struct CachedTrieEntry {
    pub node: Arc<Node>,
    pub encoded: Vec<u8>,
}

#[derive(Debug, Clone)]
struct TrieLayer {
    /// Account (state) trie nodes, keyed by unprefixed nibble path.
    account_nodes: FxHashMap<Vec<u8>, CachedTrieEntry>,
    /// Storage trie nodes, keyed by prefixed nibble path (account_nibbles ++ 0x11 ++ path).
    storage_nodes: FxHashMap<Vec<u8>, CachedTrieEntry>,
    /// FKV leaf values (both account and storage), keyed by (possibly prefixed) nibble path.
    leaf_values: FxHashMap<Vec<u8>, Vec<u8>>,
    parent: H256,
    id: usize,
}

/// In-memory cache of trie diff-layers, one per block (or per batch of blocks in full sync).
///
/// Layers form a singly-linked chain from newest to oldest via the `parent` field:
///
/// ```text
/// newest_root -> parent_1 -> parent_2 -> ... -> oldest_root -> (on-disk state)
/// ```
///
/// Each layer stores decoded `Arc<Node>` for trie nodes (eliminating `Node::decode()` on
/// cache hits) in separate account/storage maps, plus raw leaf values for the FKV shortcut.
///
/// Two commit thresholds are used in practice:
/// - **128** — regular block-by-block execution (one layer ≈ one block's trie diff).
/// - **4** — full sync / batch mode (one layer ≈ 1024 blocks ≈ 1 GB), configured via
///   `BATCH_COMMIT_THRESHOLD` in `store.rs`.
///
/// A global bloom filter is maintained across all layers to short-circuit lookups for keys
/// that don't exist in any layer, avoiding a full layer-chain walk.
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
            // TODO (issue #6345): this is coupled with DB_COMMIT_THRESHOLD in store.rs — unify them.
            commit_threshold: 128,
        }
    }
}

impl TrieLayerCache {
    /// Creates a new cache with the given commit threshold.
    ///
    /// The threshold controls how many layers accumulate before a disk flush is triggered.
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

    /// Looks up a decoded account trie node starting from the layer identified by `state_root`,
    /// walking the parent chain toward older layers.
    pub fn get_account_node(&self, state_root: H256, key: &[u8]) -> Option<Arc<Node>> {
        if !self.bloom.contains(key) {
            return None;
        }

        let mut current_state_root = state_root;

        while let Some(layer) = self.layers.get(&current_state_root) {
            if let Some(entry) = layer.account_nodes.get(key) {
                return Some(entry.node.clone());
            }
            current_state_root = layer.parent;
            if current_state_root == state_root {
                panic!("State cycle found");
            }
        }
        None
    }

    /// Looks up a decoded storage trie node starting from the layer identified by `state_root`,
    /// walking the parent chain toward older layers.
    /// `key` must be the prefixed nibble path (account_nibbles ++ 0x11 ++ trie_path).
    pub fn get_storage_node(&self, state_root: H256, key: &[u8]) -> Option<Arc<Node>> {
        if !self.bloom.contains(key) {
            return None;
        }

        let mut current_state_root = state_root;

        while let Some(layer) = self.layers.get(&current_state_root) {
            if let Some(entry) = layer.storage_nodes.get(key) {
                return Some(entry.node.clone());
            }
            current_state_root = layer.parent;
            if current_state_root == state_root {
                panic!("State cycle found");
            }
        }
        None
    }

    /// Looks up the RLP-encoded bytes for a key, checking all three maps (leaf values,
    /// account nodes, storage nodes). This is the equivalent of the old flat `get()` method
    /// and is needed by callers like `has_state_root` that call `TrieDB::get()` expecting
    /// to receive encoded node bytes.
    pub fn get_encoded(&self, state_root: H256, key: &[u8]) -> Option<Vec<u8>> {
        if !self.bloom.contains(key) {
            return None;
        }

        let mut current_state_root = state_root;

        while let Some(layer) = self.layers.get(&current_state_root) {
            if let Some(value) = layer.leaf_values.get(key) {
                return Some(value.clone());
            }
            if let Some(entry) = layer.account_nodes.get(key) {
                return Some(entry.encoded.clone());
            }
            if let Some(entry) = layer.storage_nodes.get(key) {
                return Some(entry.encoded.clone());
            }
            current_state_root = layer.parent;
            if current_state_root == state_root {
                panic!("State cycle found");
            }
        }
        None
    }

    /// Returns the state root from which to start a disk commit, using the cache's
    /// default `commit_threshold`.
    ///
    /// Used during regular block-by-block execution (threshold = 128).
    /// See [`get_commitable_with_threshold`](Self::get_commitable_with_threshold) for details.
    // TODO: use finalized hash to know when to commit
    pub fn get_commitable(&self, state_root: H256) -> Option<H256> {
        self.get_commitable_with_threshold(state_root, self.commit_threshold)
    }

    /// Walks the layer chain starting from `state_root` toward older ancestors, counting
    /// layers. When the count reaches `threshold`, returns the state root of that ancestor layer.
    ///
    /// Returns `None` if the chain has fewer than `threshold` layers (nothing to commit yet).
    pub(crate) fn get_commitable_with_threshold(
        &self,
        mut state_root: H256,
        threshold: usize,
    ) -> Option<H256> {
        let mut counter = 0;
        while let Some(layer) = self.layers.get(&state_root) {
            counter += 1;
            if counter >= threshold {
                return Some(state_root);
            }
            state_root = layer.parent;
        }
        None
    }

    /// Inserts a new diff-layer into the cache from structured `TrieCommitEntry` entries.
    ///
    /// Account entries are inserted directly. Storage entries are prefixed with the
    /// account hash (nibble-encoded with 0x11 separator) before insertion.
    pub fn put_batch(
        &mut self,
        parent: H256,
        state_root: H256,
        account_entries: Vec<TrieCommitEntry>,
        storage_entries: Vec<(H256, Vec<TrieCommitEntry>)>,
    ) {
        if parent == state_root && account_entries.is_empty() && storage_entries.is_empty() {
            return;
        } else if parent == state_root {
            tracing::trace!("parent == state_root but entries not empty");
            return;
        }
        if self.layers.contains_key(&state_root) {
            tracing::warn!("tried to insert a state_root that's already inserted");
            return;
        }

        let mut account_nodes = FxHashMap::default();
        let mut storage_nodes = FxHashMap::default();
        let mut leaf_values = FxHashMap::default();

        for entry in account_entries {
            match entry {
                TrieCommitEntry::Node {
                    path,
                    node,
                    encoded,
                } => {
                    let key = path.into_vec();
                    self.bloom.insert(&key);
                    account_nodes.insert(key, CachedTrieEntry { node, encoded });
                }
                TrieCommitEntry::LeafValue { path, value } => {
                    let key = path.into_vec();
                    self.bloom.insert(&key);
                    leaf_values.insert(key, value);
                }
            }
        }

        for (account_hash, entries) in storage_entries {
            for entry in entries {
                match entry {
                    TrieCommitEntry::Node {
                        path,
                        node,
                        encoded,
                    } => {
                        let prefixed = apply_prefix(Some(account_hash), path);
                        let key = prefixed.into_vec();
                        self.bloom.insert(&key);
                        storage_nodes.insert(key, CachedTrieEntry { node, encoded });
                    }
                    TrieCommitEntry::LeafValue { path, value } => {
                        let prefixed = apply_prefix(Some(account_hash), path);
                        let key = prefixed.into_vec();
                        self.bloom.insert(&key);
                        leaf_values.insert(key, value);
                    }
                }
            }
        }

        self.last_id += 1;
        let entry = TrieLayer {
            account_nodes,
            storage_nodes,
            leaf_values,
            parent,
            id: self.last_id,
        };
        self.layers.insert(state_root, Arc::new(entry));
    }

    /// Rebuilds the global bloom filter from scratch using all keys across all remaining layers.
    ///
    /// Called after [`commit`](Self::commit) removes layers, since the old filter may contain
    /// keys from the removed layers (producing unnecessary false positives).
    pub fn rebuild_bloom(&mut self) {
        let total_keys: usize = self
            .layers
            .values()
            .map(|layer| {
                layer.account_nodes.len() + layer.storage_nodes.len() + layer.leaf_values.len()
            })
            .sum();

        let filter = Self::create_filter(total_keys.max(BLOOM_SIZE));

        // Parallel insertion - AtomicBloomFilter allows concurrent insert via &self
        self.layers.par_iter().for_each(|(_, layer)| {
            for path in layer.account_nodes.keys() {
                filter.insert(path);
            }
            for path in layer.storage_nodes.keys() {
                filter.insert(path);
            }
            for path in layer.leaf_values.keys() {
                filter.insert(path);
            }
        });

        self.bloom = filter;
    }

    /// Removes the layer at `state_root` and all its ancestors from the cache, returning
    /// their merged trie node diffs in oldest-first order (suitable for sequential disk write).
    ///
    /// Returns the merged key-value pairs in the same flat `(Vec<u8>, Vec<u8>)` format
    /// as before, suitable for dispatch by key length in `apply_trie_updates`.
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
            .flat_map(|layer| {
                layer
                    .account_nodes
                    .into_iter()
                    .map(|(key, entry)| (key, entry.encoded))
                    .chain(
                        layer
                            .storage_nodes
                            .into_iter()
                            .map(|(key, entry)| (key, entry.encoded)),
                    )
                    .chain(layer.leaf_values)
            })
            .collect();
        Some(nodes_to_commit)
    }
}

/// [`TrieDB`] adapter that checks in-memory diff-layers ([`TrieLayerCache`]) first,
/// falling back to the on-disk trie only for keys not found in any layer.
///
/// Used by the EVM during block execution: reads see the latest uncommitted state without
/// waiting for a disk flush.
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
        let prefix_nibbles = prefix.map(|p| Nibbles::from_bytes(p.as_bytes()).append_new(17));
        Self {
            state_root,
            inner,
            db,
            prefix_nibbles,
        }
    }
}

/// Prepends an account address prefix (with an invalid nibble `17` as separator) to a
/// trie path, distinguishing storage trie entries from state trie entries in the flat
/// key-value namespace. Returns the path unchanged if `prefix` is `None` (state trie).
pub fn apply_prefix(prefix: Option<H256>, path: Nibbles) -> Nibbles {
    match prefix {
        Some(prefix) => Nibbles::from_bytes(prefix.as_bytes())
            .append_new(17)
            .concat(&path),
        None => path,
    }
}

impl TrieDB for TrieWrapper {
    fn get_node(&self, key: Nibbles) -> Result<Option<Arc<Node>>, TrieError> {
        let cached = if let Some(prefix) = &self.prefix_nibbles {
            // Storage trie — look in storage_nodes with prefixed key
            let prefixed = prefix.concat(&key);
            self.inner
                .get_storage_node(self.state_root, prefixed.as_ref())
        } else {
            // Account trie — look in account_nodes
            self.inner.get_account_node(self.state_root, key.as_ref())
        };
        Ok(cached)
    }

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
        // Check all layer cache maps (leaf values, account nodes, storage nodes).
        // This is needed because callers like `has_state_root` use `get()` to retrieve
        // the root node's encoded bytes, not just FKV leaf values.
        if let Some(value) = self.inner.get_encoded(self.state_root, key.as_ref()) {
            return Ok(Some(value));
        }
        self.db.get(key)
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        // TODO: Get rid of this.
        unimplemented!("This function should not be called");
    }
}
