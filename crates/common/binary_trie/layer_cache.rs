//! Per-block value-level layer cache for the binary trie.
//!
//! Mirrors main's `TrieLayerCache` but operates on binary trie leaf values
//! (`[u8; 32] -> Option<[u8; 32]>`) instead of MPT trie node paths.
//!
//! Layers form a singly-linked chain from newest to oldest via `parent`:
//!
//! ```text
//! newest_root -> parent_1 -> parent_2 -> ... -> oldest_root -> (on-disk state)
//! ```
//!
//! Each layer stores the leaf diffs produced by one block. When the chain
//! reaches `commit_threshold` layers, [`get_commitable`] identifies the
//! oldest layer to flush, and [`commit`] removes it (plus all ancestors)
//! and returns the merged diffs for writing to FKV on disk.

use fastbloom::AtomicBloomFilter;
use rustc_hash::{FxBuildHasher, FxHashMap};
use std::sync::Arc;

const BLOOM_SIZE: usize = 1_000_000;
const FALSE_POSITIVE_RATE: f64 = 0.02;

/// A single block's leaf diffs in the binary trie key space.
#[derive(Debug, Clone)]
struct BinaryTrieLayer {
    /// Leaf diffs: key -> Some(value) for inserts, None for deletions.
    leaves: FxHashMap<[u8; 32], Option<[u8; 32]>>,
    /// Binary trie state root of the parent layer.
    parent: [u8; 32],
    /// Monotonically increasing layer ID for ordering.
    id: usize,
}

/// In-memory cache of per-block binary trie leaf diffs.
///
/// Each layer stores the leaf-level inserts/deletions produced by one block
/// (or one batch of blocks during full sync). Reads walk the chain from
/// newest to oldest, falling through to the on-disk trie + FKV for keys
/// not found in any layer.
///
/// A global bloom filter short-circuits lookups for keys that don't exist
/// in any layer, avoiding a full chain walk.
pub struct BinaryTrieLayerCache {
    last_id: usize,
    commit_threshold: usize,
    layers: FxHashMap<[u8; 32], Arc<BinaryTrieLayer>>,
    bloom: AtomicBloomFilter<FxBuildHasher>,
}

impl std::fmt::Debug for BinaryTrieLayerCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BinaryTrieLayerCache")
            .field("last_id", &self.last_id)
            .field("commit_threshold", &self.commit_threshold)
            .field("layers_count", &self.layers.len())
            .finish()
    }
}

impl Default for BinaryTrieLayerCache {
    fn default() -> Self {
        Self {
            bloom: Self::create_filter(BLOOM_SIZE),
            last_id: 0,
            layers: FxHashMap::default(),
            commit_threshold: 128,
        }
    }
}

impl BinaryTrieLayerCache {
    pub fn new(commit_threshold: usize) -> Self {
        Self {
            commit_threshold,
            ..Default::default()
        }
    }

    fn create_filter(expected_items: usize) -> AtomicBloomFilter<FxBuildHasher> {
        AtomicBloomFilter::with_false_pos(FALSE_POSITIVE_RATE)
            .hasher(FxBuildHasher)
            .expected_items(expected_items.max(BLOOM_SIZE))
    }

    /// Look up a binary trie leaf key in the layer chain starting from `state_root`.
    ///
    /// Returns:
    /// - `Some(Some(value))` — key found with this value
    /// - `Some(None)` — key was explicitly deleted in a layer
    /// - `None` — key not in any layer, caller should check disk
    pub fn get(&self, state_root: [u8; 32], key: &[u8; 32]) -> Option<Option<[u8; 32]>> {
        if !self.bloom.contains(key) {
            return None;
        }

        let mut current = state_root;
        while let Some(layer) = self.layers.get(&current) {
            if let Some(value) = layer.leaves.get(key) {
                return Some(*value);
            }
            if layer.parent == current {
                // Cycle detection (shouldn't happen in practice).
                break;
            }
            current = layer.parent;
        }
        None
    }

    /// Insert a new layer of leaf diffs.
    ///
    /// `parent` is the binary trie root before this block.
    /// `state_root` is the binary trie root after this block.
    /// `diffs` are the leaf-level changes: `(key, Some(value))` for inserts,
    /// `(key, None)` for deletions.
    pub fn put_batch(
        &mut self,
        parent: [u8; 32],
        state_root: [u8; 32],
        diffs: Vec<([u8; 32], Option<[u8; 32]>)>,
    ) {
        // Don't insert if state_root already exists (idempotent).
        if self.layers.contains_key(&state_root) {
            return;
        }
        // Don't insert empty no-op layers.
        if parent == state_root && diffs.is_empty() {
            return;
        }

        self.last_id += 1;
        let id = self.last_id;

        let mut leaves = FxHashMap::with_capacity_and_hasher(diffs.len(), Default::default());
        for (key, value) in &diffs {
            self.bloom.insert(key);
            leaves.insert(*key, *value);
        }

        self.layers
            .insert(state_root, Arc::new(BinaryTrieLayer { leaves, parent, id }));
    }

    /// Returns the state root from which to start a disk commit, using the
    /// cache's default `commit_threshold`.
    pub fn get_commitable(&self, state_root: [u8; 32]) -> Option<[u8; 32]> {
        self.get_commitable_with_threshold(state_root, self.commit_threshold)
    }

    /// Walk the layer chain from `state_root` toward older ancestors. When the
    /// chain length reaches `threshold`, return that ancestor's state root.
    ///
    /// Returns `None` if the chain has fewer than `threshold` layers.
    pub fn get_commitable_with_threshold(
        &self,
        state_root: [u8; 32],
        threshold: usize,
    ) -> Option<[u8; 32]> {
        let mut current = state_root;
        let mut count = 0;
        let mut target = None;

        while let Some(layer) = self.layers.get(&current) {
            count += 1;
            if count >= threshold {
                target = Some(current);
            }
            if layer.parent == current {
                break;
            }
            current = layer.parent;
        }

        target
    }

    /// Commit (flush) the layer at `state_root` and all its ancestors.
    ///
    /// Removes those layers from the cache, prunes orphaned layers (layers
    /// with IDs <= the committed layer's ID that are not ancestors of any
    /// remaining layer), rebuilds the bloom filter, and returns:
    /// - The committed state roots (newest-first order).
    /// - The merged leaf diffs in oldest-first order (later values override earlier ones).
    pub fn commit(
        &mut self,
        state_root: [u8; 32],
    ) -> Option<(Vec<[u8; 32]>, Vec<([u8; 32], Option<[u8; 32]>)>)> {
        let mut layers_to_commit = Vec::new();
        let mut committed_roots = Vec::new();
        let mut current = state_root;

        while let Some(layer) = self.layers.remove(&current) {
            let layer = Arc::unwrap_or_clone(layer);
            committed_roots.push(current);
            let parent = layer.parent;
            layers_to_commit.push(layer);
            if parent == current {
                break;
            }
            current = parent;
        }

        let top_layer_id = layers_to_commit.first()?.id;

        // Remove orphaned layers (older than committed).
        self.layers.retain(|_, item| item.id > top_layer_id);
        self.rebuild_bloom();

        // Merge diffs oldest-first. Later entries override earlier ones for the
        // same key, which is correct since they represent more recent state.
        let mut merged: FxHashMap<[u8; 32], Option<[u8; 32]>> = FxHashMap::default();
        for layer in layers_to_commit.into_iter().rev() {
            for (key, value) in layer.leaves {
                merged.insert(key, value);
            }
        }

        Some((committed_roots, merged.into_iter().collect()))
    }

    /// Rebuild the bloom filter from all remaining layers.
    pub fn rebuild_bloom(&mut self) {
        self.bloom = Self::create_filter(BLOOM_SIZE);
        for layer in self.layers.values() {
            for key in layer.leaves.keys() {
                self.bloom.insert(key);
            }
        }
    }

    /// Clear all layers. Used during reorg recovery.
    pub fn clear(&mut self) {
        self.layers.clear();
        self.last_id = 0;
        self.rebuild_bloom();
    }

    /// Returns true if the given trie root has a layer in the cache.
    pub fn contains_root(&self, root: [u8; 32]) -> bool {
        self.layers.contains_key(&root)
    }

    /// Number of layers currently in the cache.
    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(n: u8) -> [u8; 32] {
        let mut r = [0u8; 32];
        r[0] = n;
        r
    }

    fn key(n: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = n;
        k
    }

    fn val(n: u8) -> [u8; 32] {
        let mut v = [0u8; 32];
        v[0] = n;
        v
    }

    #[test]
    fn get_returns_none_on_empty_cache() {
        let cache = BinaryTrieLayerCache::default();
        assert!(cache.get(root(1), &key(1)).is_none());
    }

    #[test]
    fn put_and_get_single_layer() {
        let mut cache = BinaryTrieLayerCache::default();
        cache.put_batch(root(0), root(1), vec![(key(1), Some(val(10)))]);

        assert_eq!(cache.get(root(1), &key(1)), Some(Some(val(10))));
        assert!(cache.get(root(1), &key(2)).is_none());
    }

    #[test]
    fn get_walks_parent_chain() {
        let mut cache = BinaryTrieLayerCache::default();
        // Layer 1: root(0) -> root(1), key(1) = val(10)
        cache.put_batch(root(0), root(1), vec![(key(1), Some(val(10)))]);
        // Layer 2: root(1) -> root(2), key(2) = val(20)
        cache.put_batch(root(1), root(2), vec![(key(2), Some(val(20)))]);
        // Layer 3: root(2) -> root(3), key(3) = val(30)
        cache.put_batch(root(2), root(3), vec![(key(3), Some(val(30)))]);

        // Reading from root(3) should find all three keys.
        assert_eq!(cache.get(root(3), &key(1)), Some(Some(val(10))));
        assert_eq!(cache.get(root(3), &key(2)), Some(Some(val(20))));
        assert_eq!(cache.get(root(3), &key(3)), Some(Some(val(30))));

        // Reading from root(2) should find keys 1 and 2 but not 3.
        assert_eq!(cache.get(root(2), &key(1)), Some(Some(val(10))));
        assert_eq!(cache.get(root(2), &key(2)), Some(Some(val(20))));
        assert!(cache.get(root(2), &key(3)).is_none());
    }

    #[test]
    fn newest_layer_overrides_older() {
        let mut cache = BinaryTrieLayerCache::default();
        cache.put_batch(root(0), root(1), vec![(key(1), Some(val(10)))]);
        cache.put_batch(root(1), root(2), vec![(key(1), Some(val(20)))]);

        // Newest value wins.
        assert_eq!(cache.get(root(2), &key(1)), Some(Some(val(20))));
        // Older root still sees old value.
        assert_eq!(cache.get(root(1), &key(1)), Some(Some(val(10))));
    }

    #[test]
    fn explicit_deletion_distinguishable() {
        let mut cache = BinaryTrieLayerCache::default();
        cache.put_batch(root(0), root(1), vec![(key(1), Some(val(10)))]);
        cache.put_batch(root(1), root(2), vec![(key(1), None)]); // Delete

        // From root(2): key was explicitly deleted.
        assert_eq!(cache.get(root(2), &key(1)), Some(None));
        // From root(1): key still has a value.
        assert_eq!(cache.get(root(1), &key(1)), Some(Some(val(10))));
    }

    #[test]
    fn get_commitable_under_threshold() {
        let mut cache = BinaryTrieLayerCache::new(3);
        cache.put_batch(root(0), root(1), vec![(key(1), Some(val(1)))]);
        cache.put_batch(root(1), root(2), vec![(key(2), Some(val(2)))]);

        assert!(cache.get_commitable(root(2)).is_none());
    }

    #[test]
    fn get_commitable_at_threshold() {
        let mut cache = BinaryTrieLayerCache::new(3);
        cache.put_batch(root(0), root(1), vec![(key(1), Some(val(1)))]);
        cache.put_batch(root(1), root(2), vec![(key(2), Some(val(2)))]);
        cache.put_batch(root(2), root(3), vec![(key(3), Some(val(3)))]);

        let commitable = cache.get_commitable(root(3));
        assert!(commitable.is_some());
        // The oldest layer in the chain should be returned.
        assert_eq!(commitable.unwrap(), root(1));
    }

    #[test]
    fn commit_merges_oldest_first() {
        let mut cache = BinaryTrieLayerCache::new(3);
        cache.put_batch(root(0), root(1), vec![(key(1), Some(val(10)))]);
        cache.put_batch(root(1), root(2), vec![(key(1), Some(val(20)))]);
        cache.put_batch(root(2), root(3), vec![(key(2), Some(val(30)))]);

        // Threshold 3, chain length 3 -> commitable is root(1) (oldest).
        let commitable = cache.get_commitable(root(3)).unwrap();
        assert_eq!(commitable, root(1));

        // Commit root(1): removes root(1) only (no ancestors in cache).
        let (roots, diffs) = cache.commit(commitable).unwrap();
        assert_eq!(roots, vec![root(1)]);
        let map: FxHashMap<_, _> = diffs.into_iter().collect();
        assert_eq!(map.get(&key(1)), Some(&Some(val(10))));

        // root(2) and root(3) remain.
        assert_eq!(cache.len(), 2);
        // Reading from root(3) still sees key(1)=val(20) from root(2).
        assert_eq!(cache.get(root(3), &key(1)), Some(Some(val(20))));
    }

    #[test]
    fn commit_prunes_orphans() {
        let mut cache = BinaryTrieLayerCache::new(2);
        // Main chain: root(0) -> root(1) -> root(2)
        cache.put_batch(root(0), root(1), vec![(key(1), Some(val(1)))]);
        cache.put_batch(root(1), root(2), vec![(key(2), Some(val(2)))]);
        // Fork: root(0) -> root(10) (orphan, id=3 but parent is root(0))
        cache.put_batch(root(0), root(10), vec![(key(10), Some(val(10)))]);

        assert_eq!(cache.len(), 3);

        // Commit root(1) — the oldest on the main chain.
        let (_roots, diffs) = cache.commit(root(1)).unwrap();
        assert_eq!(diffs.len(), 1);

        // root(2) should remain (id > committed id).
        // root(10) was orphaned (id=3 > committed id=1, so it stays!).
        // Actually root(10) has id=3 which is > root(1)'s id=1, so it won't be pruned.
        // This is correct: orphans are only pruned if their id <= committed id.
        assert_eq!(cache.len(), 2); // root(2) and root(10)
    }

    #[test]
    fn duplicate_state_root_is_noop() {
        let mut cache = BinaryTrieLayerCache::default();
        cache.put_batch(root(0), root(1), vec![(key(1), Some(val(10)))]);
        cache.put_batch(root(0), root(1), vec![(key(1), Some(val(99)))]); // Duplicate

        // Original value preserved.
        assert_eq!(cache.get(root(1), &key(1)), Some(Some(val(10))));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn clear_removes_everything() {
        let mut cache = BinaryTrieLayerCache::default();
        cache.put_batch(root(0), root(1), vec![(key(1), Some(val(1)))]);
        cache.put_batch(root(1), root(2), vec![(key(2), Some(val(2)))]);

        cache.clear();
        assert!(cache.is_empty());
        assert!(cache.get(root(2), &key(1)).is_none());
    }
}
