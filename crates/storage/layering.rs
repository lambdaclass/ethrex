use ethrex_common::{H256, types::BlockNumber};
use fastbloom::AtomicBloomFilter;
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap};
use std::{
    fmt,
    sync::{Arc, RwLock},
};

use ethrex_trie::{Nibbles, TrieDB, TrieError};

use crate::{
    api::{StorageBackend, tables::STATE_HISTORY},
    error::StoreError,
    journal::{JournalDecodeError, JournalEntry},
    trie::classify_trie_key,
};

const BLOOM_SIZE: usize = 1_000_000;
const FALSE_POSITIVE_RATE: f64 = 0.02;

#[derive(Debug, Clone)]
struct TrieLayer {
    nodes: FxHashMap<Vec<u8>, Vec<u8>>,
    parent: H256,
    id: usize,
    /// Number of the block whose post-state this layer represents. Used by the
    /// journal write path so a commit can record the entry under the correct
    /// block number (not the in-flight block whose insertion triggered the commit).
    block_number: BlockNumber,
    /// Hash of the block whose post-state this layer represents.
    block_hash: H256,
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
    /// Optional in-memory overlay bridging on-disk state at the cache edge `D` to the
    /// virtual state at a deep-reorg pivot. When installed, reads that miss the layer
    /// chain consult the overlay before falling through to disk. `None` in steady state.
    overlay: Option<Arc<Overlay>>,
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
            .field("overlay", &self.overlay)
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
            overlay: None,
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
            overlay: None,
            safe_commit_root,
        }
    }

    /// Installs an overlay on this cache. Subsequent reads that miss the layer chain
    /// will consult the overlay before falling through to disk. Replaces any
    /// previously-installed overlay.
    pub fn set_overlay(&mut self, overlay: Arc<Overlay>) {
        self.overlay = Some(overlay);
    }

    /// Removes any installed overlay. Idempotent.
    pub fn clear_overlay(&mut self) {
        self.overlay = None;
    }

    /// Returns a reference to the installed overlay, if any.
    pub fn overlay(&self) -> Option<&Arc<Overlay>> {
        self.overlay.as_ref()
    }

    /// Whether a reader at `state_root` should consult the installed overlay.
    ///
    /// The overlay reconstructs the pivot's state ([`Overlay::serves_root`]); the
    /// new-chain layers built on top of it during replay live in `self.layers`. Only
    /// those "consuming" roots may see overlay values ; every other reader (an
    /// eth_call/getProof at the old cache-edge `D`, or any unrelated historical root)
    /// must fall through to disk, which still holds that root's canonical state while
    /// the overlay is alive. Returns `false` when no overlay is installed.
    pub fn overlay_serves(&self, state_root: H256) -> bool {
        self.overlay
            .as_ref()
            .is_some_and(|o| state_root == o.serves_root() || self.layers.contains_key(&state_root))
    }

    /// Looks up `key` in the installed overlay. Three-state return:
    /// - `None` ; no overlay installed, or overlay does not contain the key. Caller
    ///   should fall through to disk.
    /// - `Some(None)` ; overlay says the key did not exist at the pivot. Caller
    ///   should treat as missing without consulting disk (disk still holds the OLD
    ///   chain's value).
    /// - `Some(Some(v))` ; overlay says the key had value `v` at the pivot. Caller
    ///   should return `v` without consulting disk.
    ///
    /// CF is determined by the key's length, matching `BackendTrieDB::table_for_key`.
    pub fn lookup_overlay(&self, key: &[u8]) -> Option<Option<Vec<u8>>> {
        let overlay = self.overlay.as_ref()?;
        let cf = OverlayCf::classify_by_key_length(key.len());
        overlay.lookup(cf, key)
    }

    /// Returns true if a layer with the given `state_root` is present in the cache.
    /// Used by callers (engine API, deep-reorg orchestrator) to decide whether a
    /// parent state is reachable through forward execution or requires overlay
    /// construction.
    pub fn contains(&self, state_root: H256) -> bool {
        self.layers.contains_key(&state_root)
    }

    /// Returns this cache's commit threshold. Used by the deep-reorg path so a
    /// freshly-constructed replacement cache inherits the same threshold.
    pub fn commit_threshold(&self) -> usize {
        self.commit_threshold
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
        block_number: BlockNumber,
        block_hash: H256,
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
            block_number,
            block_hash,
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

    /// Removes the layer at `state_root` and all its ancestors from the cache, returning
    /// one [`CommittedLayer`] per removed layer in oldest-first order (suitable for
    /// sequential disk write and per-block journaling).
    ///
    /// `state_root` must be a key in `self.layers` (as returned by
    /// [`get_commitable`](Self::get_commitable) /
    /// [`get_commitable_with_threshold`](Self::get_commitable_with_threshold)).
    /// If it isn't, the walk exits immediately and returns `None`.
    ///
    /// After removal, any orphaned layers (older than the committed ones) are pruned, and
    /// the bloom filter is rebuilt to remove stale entries.
    ///
    /// Normal block-by-block sync commits exactly one layer per call. Multi-layer commits
    /// are legitimate for the forkchoice-driven flush of an accumulated backlog (e.g. block
    /// import, which executes many blocks and then advances the safe-commit root once): each
    /// committed layer keeps its own block identity and `parent_state_root`, so the caller can
    /// write one journal entry per block rather than merging diffs across blocks.
    pub fn commit(&mut self, state_root: H256) -> Option<Vec<CommittedLayer>> {
        let mut layers_to_commit = vec![];
        let mut current_state_root = state_root;
        while let Some(layer) = self.layers.remove(&current_state_root) {
            let layer = Arc::unwrap_or_clone(layer);
            current_state_root = layer.parent;
            layers_to_commit.push(layer);
        }
        // `layers_to_commit` is built by walking parent links from `state_root`,
        // so `.first()` is the newest layer (the one at `state_root` itself).
        let top_layer_id = layers_to_commit.first()?.id;
        // older layers are useless
        self.layers.retain(|_, item| item.id > top_layer_id);
        self.rebuild_bloom(); // layers removed, rebuild global bloom filter.
        // Oldest-first: apply/journal in block order so per-block reverse diffs are
        // correct and newer writes overwrite older ones on disk.
        let committed = layers_to_commit
            .into_iter()
            .rev()
            .map(|layer| CommittedLayer {
                block_number: layer.block_number,
                block_hash: layer.block_hash,
                parent_state_root: layer.parent,
                nodes: layer.nodes.into_iter().collect(),
            })
            .collect();
        Some(committed)
    }
}

/// One committed layer produced by [`TrieLayerCache::commit`]: a single block's identity plus
/// the trie/flat-KV node diffs it wrote. Returned oldest-first so callers apply them in block
/// order and journal each block separately.
///
/// `parent_state_root` is the state we'd return to on rollback (this block's pre-state).
#[derive(Debug)]
pub struct CommittedLayer {
    pub block_number: BlockNumber,
    pub block_hash: H256,
    pub parent_state_root: H256,
    pub nodes: Vec<(Vec<u8>, Vec<u8>)>,
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
        // Read cascade: forward layer cache (new-chain layers above the pivot) ->
        // overlay (reverse-diff bridge to disk during deep reorgs, if installed) ->
        // on-disk state. A layer-cache hit pre-empts the overlay because a
        // side-chain write at this key supersedes the pivot value the overlay holds.
        // An overlay hit pre-empts disk because disk still reflects the OLD chain's
        // edge `D`, not the pivot.
        if let Some(value) = self.inner.get(self.state_root, key.as_ref()) {
            return Ok(Some(value));
        }
        // Overlay gate: the overlay reconstructs the pivot's state and the new-chain
        // layers built on top of it. Only a reader at a consuming root (the pivot or one
        // of those layer roots) may see it; an eth_call/getProof at the old cache-edge
        // `D` or an unrelated historical root must fall through to disk, which is
        // unchanged during the overlay window. See [`TrieLayerCache::overlay_serves`].
        if self.inner.overlay_serves(self.state_root)
            && let Some(overlay_result) = self.inner.lookup_overlay(key.as_ref())
        {
            return Ok(overlay_result);
        }
        self.db.get(key)
    }

    fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        // TODO: Get rid of this.
        unimplemented!("This function should not be called");
    }
}

// ===========================================================================
// Overlay ; in-memory aggregated reverse-diff used during deep reorgs.
// ===========================================================================

/// Identifier of which on-disk column family an [`Overlay`] entry targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayCf {
    AccountTrie,
    StorageTrie,
    AccountFlat,
    StorageFlat,
}

impl OverlayCf {
    /// Classifies an on-disk key into its CF based on length, matching the rule in
    /// `BackendTrieDB::table_for_key` / `classify_trie_key`:
    /// - `len == 65` -> `AccountFlat` (account leaf)
    /// - `len == 131` -> `StorageFlat` (storage leaf, includes 32-byte account prefix)
    /// - `len < 65` -> `AccountTrie` (non-leaf state-trie node)
    /// - otherwise -> `StorageTrie` (non-leaf storage-trie node)
    pub fn classify_by_key_length(len: usize) -> Self {
        let (is_leaf, is_account) = classify_trie_key(len);
        match (is_leaf, is_account) {
            (true, true) => OverlayCf::AccountFlat,
            (true, false) => OverlayCf::StorageFlat,
            (false, true) => OverlayCf::AccountTrie,
            (false, false) => OverlayCf::StorageTrie,
        }
    }
}

/// Errors produced while constructing an [`Overlay`] from the on-disk journal.
#[derive(Debug, thiserror::Error)]
pub enum OverlayError {
    #[error("missing journal entry for block {0}")]
    MissingEntry(BlockNumber),
    #[error(
        "journal block_hash mismatch at block {block_number}: expected {expected:?}, found {found:?}"
    )]
    HashMismatch {
        block_number: BlockNumber,
        expected: H256,
        found: H256,
    },
    #[error("invalid overlay range: from_block ({from_block}) < to_block ({to_block})")]
    InvalidRange {
        from_block: BlockNumber,
        to_block: BlockNumber,
    },
    #[error("journal decode error: {0}")]
    Decode(#[from] JournalDecodeError),
    #[error("storage error: {0}")]
    Store(#[from] StoreError),
}

/// In-memory aggregated reverse-diff bridging the on-disk state at the cache edge `D`
/// to the virtual state at a deep-reorg pivot `T-1`.
///
/// Built once per deep reorg by replaying [`STATE_HISTORY`] entries for blocks
/// `D, D-1, ..., T` in descending order. Subsequent state reads during side-chain
/// execution cascade as: new layer cache -> overlay -> on-disk state. On-disk state
/// is NOT mutated while the overlay is alive; disk stays at `D` until the first
/// new-chain commit folds the overlay and the new layer together into a single
/// atomic write (PR 3's reconciliation step).
pub struct Overlay {
    account_trie: FxHashMap<Vec<u8>, Option<Vec<u8>>>,
    storage_trie: FxHashMap<Vec<u8>, Option<Vec<u8>>>,
    account_flat: FxHashMap<Vec<u8>, Option<Vec<u8>>>,
    storage_flat: FxHashMap<Vec<u8>, Option<Vec<u8>>>,
    /// Bloom filter shared across all four CFs. A miss here lets readers skip the
    /// overlay lookup and fall through to disk without touching any map.
    bloom: AtomicBloomFilter<FxBuildHasher>,
    /// Highest block number covered by the overlay (= cache edge `D` at install time).
    from_block: BlockNumber,
    /// Lowest block number covered by the overlay (= `pivot + 1`).
    to_block: BlockNumber,
    /// State root the overlay reconstructs: the state as of `to_block - 1` (the pivot).
    /// Captured from the `parent_state_root` of the journal entry at `to_block`. Used by
    /// the read cascade to gate overlay consultation to the pivot root (and, transitively,
    /// the new-chain layer roots built on top of it) so unrelated readers of the shared
    /// cache do not get pivot values in place of the on-disk canonical state.
    /// `H256::zero()` for a default/empty overlay (never a real reconstructed root).
    serves_root: H256,
}

impl fmt::Debug for Overlay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Overlay")
            .field("account_trie_len", &self.account_trie.len())
            .field("storage_trie_len", &self.storage_trie.len())
            .field("account_flat_len", &self.account_flat.len())
            .field("storage_flat_len", &self.storage_flat.len())
            .field("from_block", &self.from_block)
            .field("to_block", &self.to_block)
            .finish()
    }
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            account_trie: FxHashMap::default(),
            storage_trie: FxHashMap::default(),
            account_flat: FxHashMap::default(),
            storage_flat: FxHashMap::default(),
            bloom: AtomicBloomFilter::with_false_pos(FALSE_POSITIVE_RATE)
                .hasher(FxBuildHasher)
                .expected_items(Self::BLOOM_INITIAL_CAPACITY),
            from_block: 0,
            to_block: 0,
            serves_root: H256::zero(),
        }
    }
}

impl Overlay {
    /// Expected-items hint used to size the bloom filter at construction time.
    /// Sized for typical reorg depths (tens to low-hundreds of blocks); the filter
    /// will still function past this count, just with a higher false-positive rate.
    const BLOOM_INITIAL_CAPACITY: usize = 64 * 1024;

    /// Builds an overlay by replaying journal entries for blocks `[to_block, from_block]`
    /// (inclusive both ends) in descending order. Each loaded entry's `block_hash` is
    /// verified against `expected_hash(n)`; a mismatch aborts with
    /// [`OverlayError::HashMismatch`].
    ///
    /// `expected_hash` is a callback that maps a height to the hash of the canonical
    /// block at that height on the chain being unwound. Returning `None` skips
    /// verification at that height (useful for tests).
    ///
    /// Within a single key, the OLDEST recorded `prev` value wins ; later inserts
    /// during the descending walk overwrite earlier ones, so the value at `to_block - 1`
    /// (whatever the oldest in-range journal entry recorded as the pre-image) is what
    /// remains after the walk.
    pub fn from_journal(
        backend: &dyn StorageBackend,
        from_block: BlockNumber,
        to_block: BlockNumber,
        expected_hash: impl Fn(BlockNumber) -> Option<H256>,
    ) -> Result<Self, OverlayError> {
        // Hard guard (not debug-only): swapped arguments would underflow `n -= 1`
        // below in release builds and loop indefinitely.
        if from_block < to_block {
            return Err(OverlayError::InvalidRange {
                from_block,
                to_block,
            });
        }
        let mut overlay = Overlay {
            from_block,
            to_block,
            ..Default::default()
        };

        // SAFETY: `StorageReadView` does not guarantee snapshot isolation on RocksDB.
        // The only writer to STATE_HISTORY is `forkchoice_update_inner` (finality
        // pruning); a concurrent FCU `delete_range` between two `.get()` calls below
        // could cause a spurious `MissingEntry`. PR 3 will install a reorg-in-progress
        // flag; while it is set, `forkchoice_update_inner` will not enter, preventing
        // pruning during overlay construction.
        let read = backend.begin_read()?;
        let mut n = from_block;
        loop {
            let bytes = read
                .get(STATE_HISTORY, &n.to_be_bytes())?
                .ok_or(OverlayError::MissingEntry(n))?;
            let entry = JournalEntry::decode(&bytes)?;
            if let Some(expected) = expected_hash(n)
                && entry.block_hash != expected
            {
                return Err(OverlayError::HashMismatch {
                    block_number: n,
                    expected,
                    found: entry.block_hash,
                });
            }
            // The entry at `to_block` unwinds `to_block -> to_block - 1`, so its
            // `parent_state_root` is the state root the overlay reconstructs (the pivot).
            if n == to_block {
                overlay.serves_root = entry.parent_state_root;
            }
            overlay.absorb(entry);
            if n == to_block {
                break;
            }
            n -= 1;
        }
        Ok(overlay)
    }

    /// Absorbs one journal entry into the overlay. Later inserts overwrite earlier
    /// ones ; combined with a descending walk in [`Self::from_journal`], this makes
    /// the OLDEST in-range entry's `prev` value win, which is the correct value at
    /// the pivot.
    fn absorb(&mut self, entry: JournalEntry) {
        for (k, v) in entry.account_trie_diff {
            self.bloom.insert(&k);
            self.account_trie.insert(k, v);
        }
        for (k, v) in entry.storage_trie_diff {
            self.bloom.insert(&k);
            self.storage_trie.insert(k, v);
        }
        for (k, v) in entry.account_flat_diff {
            self.bloom.insert(&k);
            self.account_flat.insert(k, v);
        }
        for (k, v) in entry.storage_flat_diff {
            self.bloom.insert(&k);
            self.storage_flat.insert(k, v);
        }
    }

    /// Looks up `key` in the overlay's `cf` slot. Three-state return:
    /// - `None` ; key not in overlay (caller falls through to disk).
    /// - `Some(None)` ; key was overwritten and previously didn't exist on disk
    ///   (caller treats as absent ; a rollback would delete it).
    /// - `Some(Some(v))` ; key was overwritten and previously had value `v` on disk
    ///   (caller treats as `v` ; a rollback would restore it).
    pub fn lookup(&self, cf: OverlayCf, key: &[u8]) -> Option<Option<Vec<u8>>> {
        if !self.bloom.contains(key) {
            return None;
        }
        let map = match cf {
            OverlayCf::AccountTrie => &self.account_trie,
            OverlayCf::StorageTrie => &self.storage_trie,
            OverlayCf::AccountFlat => &self.account_flat,
            OverlayCf::StorageFlat => &self.storage_flat,
        };
        map.get(key).cloned()
    }

    /// Total number of overlay entries across all four CFs. Used by tests and
    /// future observability (PR 4).
    #[allow(dead_code, reason = "consumed by tests; live target for PR 4 metrics")]
    pub fn len(&self) -> usize {
        self.account_trie.len()
            + self.storage_trie.len()
            + self.account_flat.len()
            + self.storage_flat.len()
    }

    /// Whether the overlay holds any entries.
    #[allow(dead_code, reason = "consumed by tests; live target for PR 4 metrics")]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Highest block number covered by the overlay (= cache edge `D` at install time).
    #[allow(
        clippy::wrong_self_convention,
        reason = "field accessor: name matches struct field"
    )]
    pub fn from_block(&self) -> BlockNumber {
        self.from_block
    }

    /// State root the overlay reconstructs (the pivot's state, as of `to_block - 1`).
    /// The read cascade consults the overlay only for this root and the new-chain
    /// layer roots derived from it; see [`TrieWrapper::get`].
    pub fn serves_root(&self) -> H256 {
        self.serves_root
    }

    /// Lowest block number covered by the overlay (= `pivot + 1`).
    pub fn to_block(&self) -> BlockNumber {
        self.to_block
    }

    /// Iterates every overlay entry across the four CFs as `(cf, key, value)`. Used
    /// by PR 3's reconciliation step to fold overlay-only entries into the first
    /// new-chain commit.
    pub fn iter_all_entries(
        &self,
    ) -> impl Iterator<Item = (OverlayCf, &Vec<u8>, &Option<Vec<u8>>)> {
        self.account_trie
            .iter()
            .map(|(k, v)| (OverlayCf::AccountTrie, k, v))
            .chain(
                self.storage_trie
                    .iter()
                    .map(|(k, v)| (OverlayCf::StorageTrie, k, v)),
            )
            .chain(
                self.account_flat
                    .iter()
                    .map(|(k, v)| (OverlayCf::AccountFlat, k, v)),
            )
            .chain(
                self.storage_flat
                    .iter()
                    .map(|(k, v)| (OverlayCf::StorageFlat, k, v)),
            )
    }
}

#[cfg(test)]
mod overlay_tests {
    use super::*;
    use crate::backend::in_memory::InMemoryBackend;
    use crate::journal::FlatDiff;

    fn h(b: u8) -> H256 {
        H256::repeat_byte(b)
    }

    /// Seeds N journal entries directly into STATE_HISTORY so tests can drive overlay
    /// construction without going through the full block-execution path.
    fn seed(backend: &Arc<dyn StorageBackend>, per_block: &[(BlockNumber, H256, FlatDiff)]) {
        let mut tx = backend.begin_write().unwrap();
        for (n, block_hash, diff) in per_block {
            let entry = JournalEntry {
                block_hash: *block_hash,
                parent_state_root: H256::zero(),
                account_trie_diff: diff.clone(),
                storage_trie_diff: vec![],
                account_flat_diff: vec![],
                storage_flat_diff: vec![],
            };
            tx.put(STATE_HISTORY, &n.to_be_bytes(), &entry.encode())
                .unwrap();
        }
        tx.commit().unwrap();
    }

    #[test]
    fn from_journal_loads_descending_range() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        seed(
            &backend,
            &[
                (3, h(0x03), vec![(vec![0xa], Some(vec![0x33]))]),
                (4, h(0x04), vec![(vec![0xb], Some(vec![0x44]))]),
                (5, h(0x05), vec![(vec![0xc], Some(vec![0x55]))]),
            ],
        );
        let overlay =
            Overlay::from_journal(backend.as_ref(), 5, 3, |n| Some(H256::repeat_byte(n as u8)))
                .unwrap();
        assert_eq!(overlay.len(), 3);
        assert_eq!(overlay.from_block(), 5);
        assert_eq!(overlay.to_block(), 3);
        assert_eq!(
            overlay.lookup(OverlayCf::AccountTrie, &[0xa]),
            Some(Some(vec![0x33]))
        );
        assert_eq!(
            overlay.lookup(OverlayCf::AccountTrie, &[0xb]),
            Some(Some(vec![0x44]))
        );
        assert_eq!(
            overlay.lookup(OverlayCf::AccountTrie, &[0xc]),
            Some(Some(vec![0x55]))
        );
    }

    /// Block 3 (oldest) recorded K=X. Block 5 (newest) recorded K=Y4. After
    /// descending walk, the overlay must expose K=X ; the value at the pivot
    /// (= to_block - 1 = 2).
    #[test]
    fn older_entry_wins_when_key_repeats() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        seed(
            &backend,
            &[
                (3, h(0x03), vec![(vec![0xaa], Some(b"X".to_vec()))]),
                (4, h(0x04), vec![(vec![0xaa], Some(b"Y3".to_vec()))]),
                (5, h(0x05), vec![(vec![0xaa], Some(b"Y4".to_vec()))]),
            ],
        );
        let overlay =
            Overlay::from_journal(backend.as_ref(), 5, 3, |n| Some(H256::repeat_byte(n as u8)))
                .unwrap();
        assert_eq!(
            overlay.lookup(OverlayCf::AccountTrie, &[0xaa]),
            Some(Some(b"X".to_vec())),
            "oldest reverse-diff value should win after descending walk"
        );
    }

    #[test]
    fn absent_key_passes_through_bloom() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        seed(
            &backend,
            &[(3, h(0x03), vec![(vec![0xaa], Some(vec![0x11]))])],
        );
        let overlay =
            Overlay::from_journal(backend.as_ref(), 3, 3, |n| Some(H256::repeat_byte(n as u8)))
                .unwrap();
        assert_eq!(overlay.lookup(OverlayCf::AccountTrie, &[0xff]), None);
    }

    #[test]
    fn hash_mismatch_aborts() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        seed(&backend, &[(7, h(0x07), vec![(vec![0xaa], None)])]);
        // Caller supplies the WRONG expected hash for height 7.
        let err = Overlay::from_journal(backend.as_ref(), 7, 7, |_| Some(h(0xff))).unwrap_err();
        match err {
            OverlayError::HashMismatch { block_number, .. } => assert_eq!(block_number, 7),
            other => panic!("expected HashMismatch, got {other:?}"),
        }
    }

    #[test]
    fn missing_entry_aborts() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        // Seed only block 5; ask for [5, 3] ; blocks 4 and 3 are missing.
        seed(&backend, &[(5, h(0x05), vec![])]);
        let err = Overlay::from_journal(backend.as_ref(), 5, 3, |_| None).unwrap_err();
        match err {
            OverlayError::MissingEntry(n) => assert_eq!(n, 4),
            other => panic!("expected MissingEntry, got {other:?}"),
        }
    }

    #[test]
    fn skip_verification_when_callback_returns_none() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        seed(&backend, &[(7, h(0xab), vec![(vec![0x01], None)])]);
        let overlay = Overlay::from_journal(backend.as_ref(), 7, 7, |_| None).unwrap();
        assert_eq!(overlay.lookup(OverlayCf::AccountTrie, &[0x01]), Some(None));
    }

    #[test]
    fn classify_by_key_length_matches_backend_table_routing() {
        // Spot-check the boundaries. These must agree with `classify_trie_key`
        // (account leaf at 65, storage leaf at 131, anything else routed by length).
        assert_eq!(OverlayCf::classify_by_key_length(0), OverlayCf::AccountTrie);
        assert_eq!(
            OverlayCf::classify_by_key_length(64),
            OverlayCf::AccountTrie
        );
        assert_eq!(
            OverlayCf::classify_by_key_length(65),
            OverlayCf::AccountFlat
        );
        assert_eq!(
            OverlayCf::classify_by_key_length(66),
            OverlayCf::StorageTrie
        );
        assert_eq!(
            OverlayCf::classify_by_key_length(130),
            OverlayCf::StorageTrie
        );
        assert_eq!(
            OverlayCf::classify_by_key_length(131),
            OverlayCf::StorageFlat
        );
        assert_eq!(
            OverlayCf::classify_by_key_length(132),
            OverlayCf::StorageTrie
        );
    }

    /// `serves_root` is the reconstructed pivot root, taken from the `parent_state_root`
    /// of the entry at `to_block` (the deepest in-range entry) ; NOT the `from_block`
    /// entry. Proves the capture picks the right end of the descending walk.
    #[test]
    fn from_journal_captures_serves_root_from_to_block_entry() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let pivot = h(0x77);
        let mut tx = backend.begin_write().unwrap();
        for (n, psr) in [(3u64, pivot), (4, h(0x44)), (5, h(0x55))] {
            let entry = JournalEntry {
                block_hash: h(n as u8),
                parent_state_root: psr,
                account_trie_diff: vec![(vec![n as u8], Some(vec![n as u8]))],
                storage_trie_diff: vec![],
                account_flat_diff: vec![],
                storage_flat_diff: vec![],
            };
            tx.put(STATE_HISTORY, &n.to_be_bytes(), &entry.encode())
                .unwrap();
        }
        tx.commit().unwrap();
        // Range [to_block=3, from_block=5]; serves_root must be entry-3's parent root.
        let overlay = Overlay::from_journal(backend.as_ref(), 5, 3, |_| None).unwrap();
        assert_eq!(overlay.serves_root(), pivot);
    }

    /// The overlay must only be consulted by readers at a "consuming" root ; the pivot
    /// (`serves_root`) or a new-chain layer root present in the cache. Unrelated roots
    /// (old-chain edge `D`, historical/RPC reads) must fall through to disk. Regression
    /// for the #6687 "Overlay Applies Across Roots" P1.
    #[test]
    fn overlay_serves_only_consuming_roots() {
        let pivot = h(0xaa);
        let new_chain = h(0xbb);
        let unrelated = h(0xcc);
        let parent = h(0xa9);

        let mut cache =
            TrieLayerCache::new_with_safe_commit(128, Arc::new(RwLock::new(H256::zero())));

        // No overlay installed -> never serves.
        assert!(!cache.overlay_serves(pivot));

        // Register a new-chain layer at `new_chain` (a replay commit).
        cache.put_batch(
            parent,
            new_chain,
            1,
            h(0xb1),
            vec![(Nibbles::from_bytes(&[0x01]), vec![0x02])],
        );

        // Install an overlay reconstructing the pivot state.
        let overlay = Overlay {
            serves_root: pivot,
            from_block: 5,
            to_block: 3,
            ..Default::default()
        };
        cache.set_overlay(Arc::new(overlay));

        assert!(
            cache.overlay_serves(pivot),
            "pivot root must consume the overlay"
        );
        assert!(
            cache.overlay_serves(new_chain),
            "new-chain layer root must consume the overlay"
        );
        assert!(
            !cache.overlay_serves(unrelated),
            "unrelated root must NOT see the overlay (would leak pivot state over disk)"
        );

        // Clearing the overlay disables consumption for every root.
        cache.clear_overlay();
        assert!(!cache.overlay_serves(pivot));
        assert!(!cache.overlay_serves(new_chain));
    }

    /// `lookup_overlay` is the entry point from the read cascade. It must short-circuit
    /// to `None` when no overlay is installed, regardless of key length.
    #[test]
    fn overlay_lookup_returns_none_when_no_overlay_installed() {
        let cache = TrieLayerCache::new_with_safe_commit(1, Arc::new(RwLock::new(H256::zero())));
        for key_len in [4usize, 65, 67, 131] {
            let key = vec![0xab; key_len];
            assert_eq!(
                cache.lookup_overlay(&key),
                None,
                "no overlay installed -> outer None at length {key_len}"
            );
        }
    }

    /// Installs an overlay with one entry per CF (each at the canonical length) and
    /// confirms `lookup_overlay` routes to the right map.
    #[test]
    fn overlay_lookup_classifies_cf_by_key_length() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let entry = JournalEntry {
            block_hash: h(0x01),
            parent_state_root: H256::zero(),
            account_trie_diff: vec![(vec![0x10; 4], Some(b"acct-trie".to_vec()))],
            storage_trie_diff: vec![(vec![0x20; 67], Some(b"stor-trie".to_vec()))],
            account_flat_diff: vec![(vec![0x30; 65], Some(b"acct-flat".to_vec()))],
            storage_flat_diff: vec![(vec![0x40; 131], None)],
        };
        let mut tx = backend.begin_write().unwrap();
        tx.put(STATE_HISTORY, &1u64.to_be_bytes(), &entry.encode())
            .unwrap();
        tx.commit().unwrap();
        let overlay = Overlay::from_journal(backend.as_ref(), 1, 1, |_| None).unwrap();

        let mut cache =
            TrieLayerCache::new_with_safe_commit(1, Arc::new(RwLock::new(H256::zero())));
        cache.set_overlay(Arc::new(overlay));

        assert_eq!(
            cache.lookup_overlay(&[0x10; 4]),
            Some(Some(b"acct-trie".to_vec()))
        );
        assert_eq!(
            cache.lookup_overlay(&[0x20; 67]),
            Some(Some(b"stor-trie".to_vec()))
        );
        assert_eq!(
            cache.lookup_overlay(&[0x30; 65]),
            Some(Some(b"acct-flat".to_vec()))
        );
        assert_eq!(
            cache.lookup_overlay(&[0x40; 131]),
            Some(None),
            "overlay with None means key was absent at pivot"
        );
        // Same length but different bytes ; bloom miss.
        assert_eq!(cache.lookup_overlay(&[0xee; 4]), None);
    }

    #[test]
    fn set_and_clear_overlay_round_trips() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        seed(&backend, &[(1, h(0x01), vec![(vec![0xaa], None)])]);
        let overlay = Overlay::from_journal(backend.as_ref(), 1, 1, |_| None).unwrap();

        let mut cache =
            TrieLayerCache::new_with_safe_commit(1, Arc::new(RwLock::new(H256::zero())));
        assert!(cache.overlay().is_none());
        cache.set_overlay(Arc::new(overlay));
        assert!(cache.overlay().is_some());
        cache.clear_overlay();
        assert!(cache.overlay().is_none());
        // Idempotent.
        cache.clear_overlay();
        assert!(cache.overlay().is_none());
    }

    /// `from_block == to_block == 0` is a legitimate single-block-at-genesis case
    /// and must not underflow the descending loop.
    #[test]
    fn single_entry_at_genesis() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        seed(
            &backend,
            &[(0, h(0x00), vec![(vec![0xaa], Some(vec![0x11]))])],
        );
        let overlay = Overlay::from_journal(backend.as_ref(), 0, 0, |_| None).unwrap();
        assert_eq!(overlay.len(), 1);
        assert_eq!(overlay.from_block(), 0);
        assert_eq!(overlay.to_block(), 0);
        assert_eq!(
            overlay.lookup(OverlayCf::AccountTrie, &[0xaa]),
            Some(Some(vec![0x11]))
        );
    }

    /// Swapped `from_block < to_block` must be a hard error (not a debug-only
    /// assert) so a caller mistake fires in release too. Pin the variant to guard
    /// against future error-text changes.
    #[test]
    fn swapped_args_returns_error() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let err = Overlay::from_journal(backend.as_ref(), 3, 5, |_| None).unwrap_err();
        match err {
            OverlayError::InvalidRange {
                from_block,
                to_block,
            } => {
                assert_eq!(from_block, 3);
                assert_eq!(to_block, 5);
            }
            other => panic!("expected InvalidRange, got {other:?}"),
        }
    }

    /// `Some(Some(vec![]))` (an empty-but-present pre-image) must round-trip
    /// through `absorb`/`lookup` without being confused with `Some(None)`
    /// (absent at pivot). The journal codec handles this correctly; this test
    /// guards a future codec change.
    #[test]
    fn empty_but_present_round_trips() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        seed(&backend, &[(1, h(0x01), vec![(vec![0xaa], Some(vec![]))])]);
        let overlay = Overlay::from_journal(backend.as_ref(), 1, 1, |_| None).unwrap();
        assert_eq!(
            overlay.lookup(OverlayCf::AccountTrie, &[0xaa]),
            Some(Some(vec![])),
            "empty-but-present value must NOT degrade to Some(None)"
        );
    }

    #[test]
    fn iter_all_entries_visits_each_cf() {
        // Sanity check for PR 3's reconciliation path: every CF an entry was inserted
        // into must show up in iter_all_entries, with the right CF tag.
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let entry = JournalEntry {
            block_hash: h(0x01),
            parent_state_root: H256::zero(),
            account_trie_diff: vec![(vec![0x10; 4], Some(b"at".to_vec()))],
            storage_trie_diff: vec![(vec![0x20; 67], Some(b"st".to_vec()))],
            account_flat_diff: vec![(vec![0x30; 65], None)],
            storage_flat_diff: vec![(vec![0x40; 131], Some(b"sf".to_vec()))],
        };
        let mut tx = backend.begin_write().unwrap();
        tx.put(STATE_HISTORY, &1u64.to_be_bytes(), &entry.encode())
            .unwrap();
        tx.commit().unwrap();
        let overlay = Overlay::from_journal(backend.as_ref(), 1, 1, |_| None).unwrap();

        let mut cfs: Vec<OverlayCf> = overlay.iter_all_entries().map(|(cf, _, _)| cf).collect();
        cfs.sort_by_key(|cf| match cf {
            OverlayCf::AccountTrie => 0,
            OverlayCf::StorageTrie => 1,
            OverlayCf::AccountFlat => 2,
            OverlayCf::StorageFlat => 3,
        });
        assert_eq!(
            cfs,
            vec![
                OverlayCf::AccountTrie,
                OverlayCf::StorageTrie,
                OverlayCf::AccountFlat,
                OverlayCf::StorageFlat,
            ]
        );
        assert_eq!(overlay.len(), 4);
        assert!(!overlay.is_empty());
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
            cache.put_batch(parent, root, i as u64, root, vec![(key(i), vec![i])]);
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
        cache.put_batch(b, c, 21, c, vec![(key(21), vec![21])]);
        cache.put_batch(c, b, 20, b, vec![(key(20), vec![20])]);
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
