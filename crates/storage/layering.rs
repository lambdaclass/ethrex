use ethrex_common::H256;
use fastbloom::AtomicBloomFilter;
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap};
use std::{
    fmt,
    sync::{Arc, RwLock},
};

use ethrex_trie::{Nibbles, TrieDB, TrieError};

const BLOOM_SIZE: usize = 1_000_000;
const FALSE_POSITIVE_RATE: f64 = 0.02;

#[derive(Debug, Clone)]
struct TrieLayer {
    nodes: FxHashMap<Vec<u8>, Vec<u8>>,
    parent: H256,
    id: usize,
}

/// In-memory cache of trie diff-layers, one per block (or batch in full sync), forming a
/// newest->oldest chain via `parent` down to the on-disk state.
///
/// Disk commits are gated on a canonical safe-commit root (`safe_commit_root`): a layer is
/// flushed only when that root is canonical and deep enough that it lands on the ancestor
/// walk. `H256::zero()` means "no safe commit point yet", so nothing is flushed and the
/// on-disk genesis state is never pruned. A global bloom filter short-circuits lookups for
/// keys absent from every layer.
#[derive(Clone)]
pub struct TrieLayerCache {
    /// Monotonically increasing ID for layers, starting at 1.
    /// TODO: this implementation panics on overflow
    last_id: usize,
    /// Number of layers after which we should commit to the database.
    pub(crate) commit_threshold: usize,
    layers: FxHashMap<H256, Arc<TrieLayer>>,
    /// Global bloom filter that tracks all keys across all layers.
    ///
    /// Used to avoid looking up all layers when the given path doesn't exist in any
    /// layer, thus going directly to the database.
    bloom: AtomicBloomFilter<FxBuildHasher>,
    /// The canonical safe-commit state root, computed by the Store after each forkchoice update.
    ///
    /// `H256::zero()` means "no safe commit point yet". Read by
    /// [`get_commitable`](Self::get_commitable) to gate disk commits.
    pub(crate) safe_commit_root: Arc<RwLock<H256>>,
}

impl fmt::Debug for TrieLayerCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let safe_commit = match self.safe_commit_root.read() {
            Ok(guard) => format!("{:?}", *guard),
            Err(_) => "<poisoned>".to_string(),
        };
        f.debug_struct("TrieLayerCache")
            .field("last_id", &self.last_id)
            .field("commit_threshold", &self.commit_threshold)
            .field("layers", &self.layers)
            .field("bloom", &"AtomicBloomFilter")
            .field("safe_commit_root", &safe_commit)
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
            safe_commit_root: Arc::new(RwLock::new(H256::zero())),
        }
    }
}

impl TrieLayerCache {
    /// Creates a new cache with the given commit threshold and a shared safe-commit-root cell.
    ///
    /// The `safe_commit_root` Arc is shared with [`Store`](crate::Store) so that the Store
    /// can update the cell without replacing the cache Arc.
    /// `H256::zero()` in the cell means "no safe commit point yet".
    pub fn new_with_safe_commit(
        commit_threshold: usize,
        safe_commit_root: Arc<RwLock<H256>>,
    ) -> Self {
        Self {
            bloom: Self::create_filter(BLOOM_SIZE),
            last_id: 0,
            layers: Default::default(),
            commit_threshold,
            safe_commit_root,
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

    /// Determines whether a disk commit should happen by checking whether the safe-commit root
    /// appears on the ancestor chain starting from `parent_state_root`.
    ///
    /// Returns `Some(safe_commit_root)` when the root is found on the ancestor walk; `None`
    /// when the cell is zero, poisoned, or the root is not on the walk. The bounded-walk
    /// cycle guard caps the walk at `layers.len()` steps to ensure termination.
    pub fn get_commitable(&self, parent_state_root: H256) -> Option<H256> {
        // (a) Read the safe-commit root; a poisoned lock is treated as "not ready".
        let safe_root = *self.safe_commit_root.read().ok()?;
        // (b) Zero means no safe commit point yet; commit nothing.
        if safe_root.is_zero() {
            return None;
        }
        // (c) The executed parent IS the safe-commit root; commit immediately.
        if parent_state_root == safe_root {
            return Some(safe_root);
        }
        // (d) Walk the layer parent-chain from parent_state_root looking for safe_root.
        let mut current = parent_state_root;
        let mut steps = 0usize;
        let max_steps = self.layers.len();
        while let Some(layer) = self.layers.get(&current) {
            if current == safe_root {
                return Some(safe_root);
            }
            let next = layer.parent;
            // Cycle guard: if walking would return to the walk start, stop.
            if next == parent_state_root {
                return None;
            }
            steps += 1;
            // Bounded-walk safeguard: a mid-chain cycle (e.g. B→C→B) would not be
            // caught by the start-of-walk guard above. Capping at layers.len() steps
            // ensures the loop always terminates.
            if steps > max_steps {
                return None;
            }
            current = next;
        }
        // (e) Reached chain bottom (root not in layers / already on disk) without matching safe_root.
        None
    }

    /// Depth-only commit gate for batch execution (full sync / block import).
    ///
    /// Walks the parent chain from `state_root`, counting layers, and returns the state root of
    /// the layer that is `threshold` layers deep — committing purely by depth, ignoring the
    /// canonical [`safe_commit_root`](Self::safe_commit_root) cell.
    ///
    /// Used only in batch mode, where the node extends a single canonical chain (full sync and
    /// `import` never execute competing forks), so the non-canonical-commit hazard that the
    /// canonical gate guards against cannot occur. The canonical gate keys on the `head - 128`
    /// safe-commit root, which never lands on a batch layer boundary (~1024 blocks apart), so it
    /// would never flush during batch execution; this depth gate bounds memory instead.
    pub(crate) fn get_commitable_by_depth(
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
    /// Called after [`commit`](Self::commit) removes layers, since the old filter may contain
    /// keys from the removed layers (producing unnecessary false positives).
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

    /// Removes the layer at `state_root` (a key returned by [`get_commitable`](Self::get_commitable))
    /// and all older ancestors, returning their merged diffs oldest-first for sequential disk write.
    /// Returns `None` if `state_root` is not a layer.
    ///
    /// The `retain(id > top_layer_id)` step keeps every layer newer than `state_root` in memory, so
    /// speculative not-yet-canonical state is never dropped; the bloom filter is then rebuilt.
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
    use std::collections::HashSet;
    use std::sync::{Arc, RwLock};

    use ethrex_common::H256;
    use ethrex_trie::Nibbles;

    use super::TrieLayerCache;

    /// Build an `H256` from a single byte, all other bytes zero.
    fn h256(b: u8) -> H256 {
        let mut bytes = [0u8; 32];
        bytes[31] = b;
        H256(bytes)
    }

    /// A dummy trie key, distinct per layer so `put_batch`'s empty-block guard does not skip it.
    fn key(b: u8) -> Nibbles {
        Nibbles::from_bytes(&[b; 32])
    }

    /// Build a linear chain of N layers on top of an on-disk floor (`H256::zero()`),
    /// returning the roots in order `[root_1, ..., root_n]` with `root_{i+1}.parent == root_i`.
    fn build_chain(cache: &mut TrieLayerCache, n: u8) -> Vec<H256> {
        let mut roots = Vec::with_capacity(n as usize);
        let mut parent = H256::zero();
        for i in 1..=n {
            let root = h256(i);
            cache.put_batch(parent, root, vec![(key(i), vec![i])]);
            roots.push(root);
            parent = root;
        }
        roots
    }

    fn cache_with_cell(threshold: usize, root: H256) -> (TrieLayerCache, Arc<RwLock<H256>>) {
        let cell = Arc::new(RwLock::new(root));
        let cache = TrieLayerCache::new_with_safe_commit(threshold, Arc::clone(&cell));
        (cache, cell)
    }

    /// (a) Zero cell means "no safe commit point yet" -> get_commitable returns None.
    #[test]
    fn zero_cell_yields_none() {
        let (mut cache, _cell) = cache_with_cell(4, H256::zero());
        let roots = build_chain(&mut cache, 5);
        assert_eq!(cache.get_commitable(*roots.last().unwrap()), None);
    }

    /// (b) Safe root on the parent walk -> get_commitable returns Some(safe_root).
    #[test]
    fn safe_root_on_walk_yields_some() {
        let safe = h256(2);
        let (mut cache, _cell) = cache_with_cell(4, safe);
        let roots = build_chain(&mut cache, 4);
        // Walk from L4: L4 -> L3 -> L2 (== safe) -> Some(L2).
        assert_eq!(cache.get_commitable(roots[3]), Some(safe));
    }

    /// (c) parent_state_root == safe root -> immediate Some, no walk needed.
    #[test]
    fn parent_equals_safe_root_yields_some() {
        let roots = {
            let (mut cache, _cell) = cache_with_cell(4, H256::zero());
            build_chain(&mut cache, 3)
        };
        let l3 = roots[2];
        let (mut cache, _cell) = cache_with_cell(4, l3);
        build_chain(&mut cache, 3);
        assert_eq!(cache.get_commitable(l3), Some(l3));
    }

    /// (d) Safe root not an ancestor (never inserted as a layer) -> None, regardless of depth.
    #[test]
    fn safe_root_not_ancestor_yields_none() {
        let (mut cache, _cell) = cache_with_cell(4, h256(99));
        // 6 layers (> threshold) so an old depth-only path would have fired.
        let roots = build_chain(&mut cache, 6);
        assert_eq!(cache.get_commitable(*roots.last().unwrap()), None);
    }

    /// (e) Cycle guard: a B -> C -> B link must terminate and return None.
    #[test]
    fn cycle_guard_terminates() {
        let b = h256(20);
        let c = h256(21);
        let (mut cache, _cell) = cache_with_cell(4, h256(99));
        // Insert C (parent B) then B (parent C); neither key pre-exists, so both insert,
        // forming the cycle B <-> C. The safe root (h256(99)) is absent on purpose.
        cache.put_batch(b, c, vec![(key(21), vec![21])]);
        cache.put_batch(c, b, vec![(key(20), vec![20])]);
        // Walking from C must terminate (start-of-walk + bounded-walk guards) and yield None.
        assert_eq!(cache.get_commitable(c), None);
    }

    /// (f) commit(safe_root) removes the safe layer and all older ones, retaining layers above it.
    #[test]
    fn commit_retains_layers_above_safe_root() {
        let safe = h256(2);
        let (mut cache, _cell) = cache_with_cell(4, safe);
        let roots = build_chain(&mut cache, 4);
        let (l3, l4) = (roots[2], roots[3]);
        assert_eq!(cache.get_commitable(roots[3]), Some(safe));

        cache.commit(safe);
        let remaining: HashSet<H256> = cache.layers.keys().copied().collect();
        let expected: HashSet<H256> = [l3, l4].into_iter().collect();
        assert_eq!(
            remaining, expected,
            "commit(safe) must retain only the layers above it"
        );
    }

    /// (g) Memory bound: after building > threshold layers and committing at a safe root that
    /// keeps the chain bounded, the retained layer count stays <= commit_threshold + 1.
    #[test]
    fn memory_bound_after_commit() {
        let threshold = 4usize;
        // Build threshold + 3 = 7 layers; set the safe root `threshold` below the tip.
        let n = (threshold + 3) as u8;
        let safe = h256(n - threshold as u8); // h256(3): leaves layers 4..=7 above it
        let (mut cache, _cell) = cache_with_cell(threshold, safe);
        let roots = build_chain(&mut cache, n);
        let tip = *roots.last().unwrap();

        let commitable = cache.get_commitable(tip).expect("safe root on the walk");
        cache.commit(commitable);
        assert!(
            cache.layers.len() <= threshold + 1,
            "retained layers ({}) must stay within commit_threshold + 1 ({})",
            cache.layers.len(),
            threshold + 1
        );
    }

    /// Why live block-by-block execution must NOT use the depth gate: with nothing canonicalized
    /// (safe_commit cell ZERO), the depth gate would flush a non-canonical layer and prune genesis
    /// -> the "post-state for block 0 absent" wedge. The canonical gate commits nothing instead.
    ///
    /// Wedge simulation: non-canonical newPayload layers pile up but nothing is canonicalized,
    /// so the safe_commit cell stays ZERO and never advances.
    #[test]
    fn live_canonical_gate_holds_while_depth_gate_would_commit() {
        // threshold = 4, safe_commit cell = ZERO (nothing canonicalized).
        let (mut cache, _cell) = cache_with_cell(4, H256::zero());
        // Linear chain L1 <- L2 <- L3 <- L4 <- L5 (distinct keys so guards pass).
        let roots = build_chain(&mut cache, 5);
        let l5 = *roots.last().unwrap();

        // Depth gate: commits a layer at depth 4 -> would prune genesis on the path-keyed disk
        // root if used in live mode. This is why live mode uses the canonical gate below.
        assert!(
            cache.get_commitable_by_depth(l5, 4).is_some(),
            "depth-only gate commits at depth 4 regardless of canonicality"
        );

        // Canonical gate (live mode): safe_commit cell is zero, so nothing is committed and
        // genesis is preserved.
        assert_eq!(
            cache.get_commitable(l5),
            None,
            "canonical gate must commit nothing while safe_commit is zero (the wedge fix)"
        );
    }

    /// Batch execution (full sync / import) must still flush even when no FCU has advanced the
    /// safe-commit cell. The canonical gate stays parked at zero (import does not FCU until the
    /// end; full sync's `head - 128` root never lands on a ~1024-block batch boundary), so batch
    /// mode commits by depth instead -> memory stays bounded and state is durable across restart.
    #[test]
    fn batch_depth_gate_flushes_without_safe_commit() {
        // safe_commit cell = ZERO, as during bulk import before the terminal FCU.
        let (mut cache, _cell) = cache_with_cell(4, H256::zero());
        // Five batch layers stacked (each stands in for ~1024 blocks in real batch mode).
        let roots = build_chain(&mut cache, 5);
        let tip = *roots.last().unwrap();

        // Canonical gate would never flush here -> unbounded memory (the regression iovoid flagged).
        assert_eq!(
            cache.get_commitable(tip),
            None,
            "canonical gate never flushes batch layers while safe_commit is zero"
        );

        // Depth gate (batch mode) flushes the layer BATCH_COMMIT_THRESHOLD deep: root_2 sits 4
        // layers below the tip (tip=root_5).
        assert_eq!(
            cache.get_commitable_by_depth(tip, 4),
            Some(roots[1]),
            "batch depth gate must flush the layer 4 deep, bounding memory"
        );
    }
}
