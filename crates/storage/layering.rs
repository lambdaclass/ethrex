use ethrex_common::H256;
use fastbloom::AtomicBloomFilter;
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap};
use std::{fmt, sync::Arc};

use ethrex_trie::{Nibbles, TrieDB, TrieError};

const BLOOM_SIZE: usize = 1_000_000;
const FALSE_POSITIVE_RATE: f64 = 0.02;
/// Number of commits between full bloom filter rebuilds.
///
/// Deferring the rebuild is safe because the bloom is only ever a superset of the live
/// key set: keys of removed layers linger as potential false positives (a wasted layer
/// walk before falling through to disk), but lookups can never produce a false negative.
/// Periodically rebuilding only sheds those stale keys to bound the false-positive rate.
pub(crate) const BLOOM_REBUILD_INTERVAL: usize = 16;

#[derive(Debug, Clone)]
struct TrieLayer {
    nodes: FxHashMap<Vec<u8>, Vec<u8>>,
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
/// Each layer stores the trie node diffs produced by one block (regular sync) or one batch
/// of ~1024 blocks (full sync). When the chain reaches `commit_threshold` layers,
/// [`get_commitable`](Self::get_commitable) identifies the layer to flush, and
/// [`commit`](Self::commit) removes it (plus all ancestors) and returns the merged key-values
/// for writing to RocksDB.
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
    ///
    /// Wrapped in `Arc` so cloning the cache (the RCU pattern in `store.rs`) bumps a
    /// refcount instead of deep-copying the bit array. Inserting through the shared
    /// filter is safe: `AtomicBloomFilter` supports concurrent insert via `&self`, and
    /// added keys only make the bloom a larger superset for older cache snapshots.
    bloom: Arc<AtomicBloomFilter<FxBuildHasher>>,
    /// Commits since the bloom was last rebuilt, used to schedule periodic rebuilds.
    commits_since_rebuild: usize,
    /// Keys belonging to removed layers since the last rebuild (stale bloom entries).
    stale_keys_since_rebuild: usize,
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
            bloom: Arc::new(Self::create_filter(BLOOM_SIZE)),
            last_id: 0,
            layers: Default::default(),
            // TODO (issue #6345): this is coupled with DB_COMMIT_THRESHOLD in store.rs — unify them.
            commit_threshold: 128,
            commits_since_rebuild: 0,
            stale_keys_since_rebuild: 0,
        }
    }
}

impl TrieLayerCache {
    /// Creates a new cache with the given commit threshold.
    ///
    /// The threshold controls how many layers accumulate before a disk flush is triggered.
    pub fn new(commit_threshold: usize) -> Self {
        Self {
            bloom: Arc::new(Self::create_filter(BLOOM_SIZE)),
            last_id: 0,
            layers: Default::default(),
            commit_threshold,
            commits_since_rebuild: 0,
            stale_keys_since_rebuild: 0,
        }
    }

    fn create_filter(expected_items: usize) -> AtomicBloomFilter<FxBuildHasher> {
        AtomicBloomFilter::with_false_pos(FALSE_POSITIVE_RATE)
            .hasher(FxBuildHasher)
            .expected_items(expected_items.max(BLOOM_SIZE))
    }

    /// Looks up a trie node `key` starting from the layer identified by `state_root`,
    /// walking the parent chain toward older layers.
    ///
    /// Returns `Some(value)` from the first (newest) layer that contains the key, or `None`
    /// if no layer has it. A bloom filter is checked first to skip the walk entirely when the
    /// key is guaranteed absent from all layers (callers then fall through to the on-disk trie).
    pub fn get(&self, state_root: H256, key: &[u8]) -> Option<Vec<u8>> {
        // Fast check to know if any layer may contain the given key.
        // We can only be certain it doesn't exist, but if it returns true it may or may not exist (false positive).
        if !self.bloom.contains(key) {
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
    ///
    /// This function is used to determine when to trigger a disk commit. We consider a layer "committable"
    /// when it has at least `threshold` newer layers on top of it, ensuring that we only commit sufficiently
    /// old layers and keep recent ones in memory for fast access.
    ///
    /// Having a threshold allows both customizing the commit frequency (e.g. full sync vs regular block execution)
    /// and avoiding edge cases where there could, theoretically, be a cycle in the layer change.
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

    /// Inserts a new diff-layer into the cache, keyed by `state_root` and pointing to `parent`.
    ///
    /// In regular sync each call adds one block's trie diffs. In full sync (batch mode), each
    /// call adds diffs for an entire batch of ~1024 blocks.
    ///
    /// No-ops if `parent == state_root` (empty block with no state change), or if `state_root`
    /// is already present (duplicate insertion guard).
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

    /// Rebuilds the global bloom filter from scratch using all keys across all remaining layers.
    ///
    /// Called periodically from [`commit`](Self::commit), since the old filter may contain
    /// keys from removed layers (producing unnecessary false positives).
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

        self.bloom = Arc::new(filter);
        self.commits_since_rebuild = 0;
        self.stale_keys_since_rebuild = 0;
    }

    /// Removes the layer at `state_root` and all its ancestors from the cache, returning
    /// their merged trie node diffs in oldest-first order (suitable for sequential disk write).
    ///
    /// `state_root` must be a key in `self.layers` (as returned by
    /// [`get_commitable`](Self::get_commitable) /
    /// [`get_commitable_with_threshold`](Self::get_commitable_with_threshold)).
    /// If it isn't, the walk exits immediately and returns `None`.
    ///
    /// After removal, any orphaned layers (older than the committed ones) are pruned, and
    /// the bloom filter is periodically rebuilt to shed stale entries
    /// (see [`BLOOM_REBUILD_INTERVAL`]).
    pub fn commit(&mut self, state_root: H256) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut layers_to_commit = vec![];
        let mut current_state_root = state_root;
        while let Some(layer) = self.layers.remove(&current_state_root) {
            let layer = Arc::unwrap_or_clone(layer);
            current_state_root = layer.parent;
            layers_to_commit.push(layer);
        }
        let top_layer_id = layers_to_commit.first()?.id;
        let mut removed_keys: usize = layers_to_commit.iter().map(|layer| layer.nodes.len()).sum();
        // older layers are useless
        self.layers.retain(|_, item| {
            let keep = item.id > top_layer_id;
            if !keep {
                removed_keys += item.nodes.len();
            }
            keep
        });
        // The bloom now holds stale keys for the removed layers. That is safe (it stays a
        // superset of the live key set: only false positives, never false negatives), so
        // instead of paying a full rebuild on every commit we only rebuild periodically,
        // or sooner if stale keys outnumber live ones (keeps batch-mode commits, which
        // remove very large layers, from saturating the filter).
        self.commits_since_rebuild += 1;
        self.stale_keys_since_rebuild += removed_keys;
        let live_keys: usize = self.layers.values().map(|layer| layer.nodes.len()).sum();
        if self.commits_since_rebuild >= BLOOM_REBUILD_INTERVAL
            || self.stale_keys_since_rebuild > live_keys
        {
            self.rebuild_bloom();
        }
        let nodes_to_commit = layers_to_commit
            .into_iter()
            .rev()
            .flat_map(|layer| layer.nodes)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn root(n: u64) -> H256 {
        H256::from_low_u64_be(n)
    }

    /// Generates key-values unique to the given seed, so each layer has disjoint keys.
    fn layer_keys(seed: u64) -> Vec<(Nibbles, Vec<u8>)> {
        (0..32u64)
            .map(|i| {
                (
                    Nibbles::from_bytes(&[seed.to_be_bytes(), i.to_be_bytes()].concat()),
                    vec![seed as u8, i as u8],
                )
            })
            .collect()
    }

    /// Every key of every layer still in the cache must be retrievable starting from the
    /// newest root: the bloom may hold stale keys, but must never produce a false negative.
    fn assert_live_keys_hit(cache: &TrieLayerCache, newest_root: H256) {
        let mut current = newest_root;
        while let Some(layer) = cache.layers.get(&current) {
            for (key, value) in &layer.nodes {
                assert_eq!(cache.get(newest_root, key).as_ref(), Some(value));
            }
            current = layer.parent;
        }
    }

    #[test]
    fn bloom_rebuild_is_deferred_and_keeps_superset_property() {
        const THRESHOLD: usize = 32;
        let mut cache = TrieLayerCache::new(THRESHOLD);

        // Build the initial chain up to the commit threshold.
        for n in 1..=THRESHOLD as u64 {
            cache.put_batch(root(n - 1), root(n), layer_keys(n));
        }

        let mut saw_deferred_commit = false;
        let mut saw_rebuild = false;
        // Keep adding layers and committing, well past the rebuild interval.
        let first = THRESHOLD as u64 + 1;
        let last = THRESHOLD as u64 + 2 * BLOOM_REBUILD_INTERVAL as u64;
        for n in first..=last {
            cache.put_batch(root(n - 1), root(n), layer_keys(n));
            let commitable = cache
                .get_commitable(root(n))
                .expect("a layer should be committable");
            assert!(cache.commit(commitable).is_some());
            if cache.commits_since_rebuild > 0 {
                saw_deferred_commit = true;
            } else {
                saw_rebuild = true;
            }
            assert_live_keys_hit(&cache, root(n));
        }
        assert!(
            saw_deferred_commit,
            "expected at least one commit to defer the bloom rebuild"
        );
        assert!(saw_rebuild, "expected the bloom to be rebuilt eventually");
    }
}
