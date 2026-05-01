use ethrex_common::{H256, types::BlockNumber};
use fastbloom::AtomicBloomFilter;
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap};
use std::{fmt, sync::Arc};

use ethrex_trie::{Nibbles, TrieDB, TrieError};

use crate::{
    api::{
        StorageBackend,
        tables::{ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, STATE_HISTORY, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES},
    },
    error::StoreError,
    journal::{JournalDecodeError, JournalEntry},
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
    /// block number (not the in-flight block whose insertion *triggered* the
    /// commit).
    block_number: BlockNumber,
    /// Hash of the block whose post-state this layer represents.
    block_hash: H256,
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
    bloom: AtomicBloomFilter<FxBuildHasher>,
    /// Optional in-memory overlay bridging the on-disk state at the cache edge to
    /// a deeper pivot during a deep reorg. Reads that miss the layer cache consult
    /// the overlay before falling through to disk. `None` in steady state.
    overlay: Option<Arc<Overlay>>,
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
            overlay: None,
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
            overlay: None,
        }
    }

    /// Installs an overlay on this cache. Subsequent reads that miss the layer
    /// chain will consult the overlay before falling through to disk. Replaces
    /// any previously-installed overlay.
    #[allow(dead_code, reason = "consumed by Section 8 (deep-reorg apply path)")]
    pub fn set_overlay(&mut self, overlay: Arc<Overlay>) {
        self.overlay = Some(overlay);
    }

    /// Removes any installed overlay. Called after the first new-chain commit
    /// reconciles the overlay into disk (Section 9).
    #[allow(dead_code, reason = "consumed by Section 9 (overlay reconciliation)")]
    pub fn clear_overlay(&mut self) {
        self.overlay = None;
    }

    /// Returns a reference to the installed overlay, if any. Used by tests
    /// and by the reconciliation path to fold the overlay into the first
    /// new-chain commit.
    #[allow(dead_code, reason = "consumed by Section 9 (overlay reconciliation) and tests")]
    pub fn overlay(&self) -> Option<&Arc<Overlay>> {
        self.overlay.as_ref()
    }

    /// Looks up `key` in the installed overlay. Returns:
    /// - `None` if no overlay is installed, or the overlay does not contain the key.
    ///   Caller should fall through to on-disk state.
    /// - `Some(None)` if the overlay holds the key with absence — the key did not
    ///   exist at the pivot. Caller should treat as missing without consulting disk.
    /// - `Some(Some(v))` if the overlay holds the key with value `v`. Caller should
    ///   return `v` without consulting disk.
    ///
    /// The CF is determined by the key's length, matching `BackendTrieDB::table_for_key`:
    ///  `len == 65` → account flat-KV; `len == 131` → storage flat-KV;
    ///  `len < 65` → account trie node; otherwise storage trie node.
    pub fn lookup_overlay(&self, key: &[u8]) -> Option<Option<Vec<u8>>> {
        let overlay = self.overlay.as_ref()?;
        let cf = OverlayCf::classify_by_key_length(key.len());
        overlay.lookup(cf, key)
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

    /// Returns `true` if a layer with the given `state_root` is present in the cache.
    /// Used by the engine API to decide whether a `newPayload`'s parent state is
    /// reachable through forward execution (eager path) or whether the payload must be
    /// stashed pending a deeper reorg (deferred path returning `ACCEPTED`).
    pub fn contains(&self, state_root: H256) -> bool {
        self.layers.contains_key(&state_root)
    }

    /// Returns the commit threshold of this cache. Used by the deep-reorg
    /// path so a freshly-constructed replacement cache inherits the same
    /// threshold (carrying batch-mode vs regular-mode configuration).
    pub fn commit_threshold(&self) -> usize {
        self.commit_threshold
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
    /// the committed block's identity plus their merged trie node diffs in oldest-first order
    /// (suitable for sequential disk write).
    ///
    /// `state_root` must be a key in `self.layers` (as returned by
    /// [`get_commitable`](Self::get_commitable) /
    /// [`get_commitable_with_threshold`](Self::get_commitable_with_threshold)).
    /// If it isn't, the walk exits immediately and returns `None`.
    ///
    /// After removal, any orphaned layers (older than the committed ones) are pruned, and
    /// the bloom filter is rebuilt to remove stale entries.
    ///
    /// Returns `(block_number, block_hash, parent_state_root, merged_nodes)` of the layer at
    /// `state_root`. `parent_state_root` is the state root we'd return to on rollback (the
    /// committed block's pre-state). In normal operation only one layer is removed; ancestors
    /// are evicted as orphans without contributing to the merged nodes (caught by the `id`
    /// retain below).
    pub fn commit(
        &mut self,
        state_root: H256,
    ) -> Option<CommitResult> {
        let mut layers_to_commit = vec![];
        let mut current_state_root = state_root;
        while let Some(layer) = self.layers.remove(&current_state_root) {
            let layer = Arc::unwrap_or_clone(layer);
            current_state_root = layer.parent;
            layers_to_commit.push(layer);
        }
        let top_layer = layers_to_commit.first()?;
        let top_layer_id = top_layer.id;
        let committed_block_number = top_layer.block_number;
        let committed_block_hash = top_layer.block_hash;
        let committed_parent_state_root = top_layer.parent;
        // older layers are useless
        self.layers.retain(|_, item| item.id > top_layer_id);
        self.rebuild_bloom(); // layers removed, rebuild global bloom filter.
        let nodes_to_commit = layers_to_commit
            .into_iter()
            .rev()
            .flat_map(|layer| layer.nodes)
            .collect();
        Some(CommitResult {
            block_number: committed_block_number,
            block_hash: committed_block_hash,
            parent_state_root: committed_parent_state_root,
            nodes: nodes_to_commit,
        })
    }
}

/// Output of [`TrieLayerCache::commit`]: the identity of the committed block plus the merged
/// trie node updates to write to disk.
#[derive(Debug, Default)]
pub struct CommitResult {
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
        // Read cascade: layer cache (forward layers above the pivot) → overlay
        // (reverse-diff bridging disk → pivot during deep reorgs) → disk.
        // A layer-cache hit pre-empts the overlay because side-chain writes
        // shadow the pivot value for any key the new chain has touched.
        if let Some(value) = self.inner.get(self.state_root, key.as_ref()) {
            return Ok(Some(value));
        }
        if let Some(overlay_result) = self.inner.lookup_overlay(key.as_ref()) {
            // Overlay says: key had value `v` at pivot, OR key was absent at
            // pivot. Either way, do NOT consult disk (disk holds the OLD
            // chain's value, not the pivot value).
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
// Overlay — in-memory aggregated reverse-diff used during deep reorgs.
// ===========================================================================

/// Identifier of which on-disk column family an [`Overlay`] entry targets.
/// Returned by classifier helpers; used by callers to route a key to the right
/// internal map without re-doing the length classification.
#[allow(dead_code, reason = "consumed by the read cascade in Section 7 / reorg apply in Section 8")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayCf {
    AccountTrie,
    StorageTrie,
    AccountFlat,
    StorageFlat,
}

impl OverlayCf {
    /// Maps `OverlayCf` to its column-family name.
    #[allow(dead_code, reason = "consumed by Section 9 (overlay reconciliation)")]
    pub fn table(self) -> &'static str {
        match self {
            OverlayCf::AccountTrie => ACCOUNT_TRIE_NODES,
            OverlayCf::StorageTrie => STORAGE_TRIE_NODES,
            OverlayCf::AccountFlat => ACCOUNT_FLATKEYVALUE,
            OverlayCf::StorageFlat => STORAGE_FLATKEYVALUE,
        }
    }

    /// Classifies an on-disk key into its CF based on length, matching the
    /// rules in `BackendTrieDB::table_for_key`:
    /// - `len == 65` → `AccountFlat` (account leaf)
    /// - `len == 131` → `StorageFlat` (storage leaf, including 32-byte account prefix)
    /// - `len < 65` → `AccountTrie` (non-leaf state-trie node)
    /// - otherwise → `StorageTrie` (non-leaf storage-trie node)
    pub fn classify_by_key_length(len: usize) -> Self {
        let is_leaf = len == 65 || len == 131;
        let is_account = len <= 65;
        match (is_leaf, is_account) {
            (true, true) => OverlayCf::AccountFlat,
            (true, false) => OverlayCf::StorageFlat,
            (false, true) => OverlayCf::AccountTrie,
            (false, false) => OverlayCf::StorageTrie,
        }
    }
}

/// Errors produced while constructing an [`Overlay`] from the on-disk journal.
#[allow(dead_code, reason = "consumed by Section 8 (deep-reorg apply path)")]
#[derive(Debug, thiserror::Error)]
pub enum OverlayError {
    #[error("missing journal entry for block {0}")]
    MissingEntry(BlockNumber),
    #[error("journal block_hash mismatch at block {block_number}: expected {expected:?}, found {found:?}")]
    HashMismatch {
        block_number: BlockNumber,
        expected: H256,
        found: H256,
    },
    #[error("journal decode error: {0}")]
    Decode(#[from] JournalDecodeError),
    #[error("storage error: {0}")]
    Store(#[from] StoreError),
}

/// In-memory aggregated reverse-diff bridging the on-disk state at the cache
/// edge `D` to the virtual state at the deep-reorg pivot `T-1`.
///
/// Built once per deep reorg by replaying [`STATE_HISTORY`] entries for blocks
/// `D, D-1, ..., T` in descending order. Subsequent state reads during
/// side-chain execution cascade as: new layer cache → overlay → on-disk state.
/// On-disk state is NOT mutated while the overlay is alive — disk stays at `D`
/// until the first new-chain commit folds the overlay and the new layer
/// together into a single atomic write.
pub struct Overlay {
    account_trie: FxHashMap<Vec<u8>, Option<Vec<u8>>>,
    storage_trie: FxHashMap<Vec<u8>, Option<Vec<u8>>>,
    account_flat: FxHashMap<Vec<u8>, Option<Vec<u8>>>,
    storage_flat: FxHashMap<Vec<u8>, Option<Vec<u8>>>,
    /// Bloom filter shared across all four CFs. Populated as entries are added.
    /// A miss here lets readers skip overlay lookup and fall through to disk.
    bloom: AtomicBloomFilter<FxBuildHasher>,
    /// Highest block number covered by the overlay (= cache edge `D` at install time).
    /// Used by Section 9's reconciliation to issue `delete_range` for obsolete
    /// old-chain journal entries.
    from_block: BlockNumber,
    /// Lowest block number covered by the overlay (= pivot+1, where pivot is
    /// `to_block - 1`). The first new-chain block at this height matches
    /// `to_block`; reconciliation uses this for the same range computation.
    to_block: BlockNumber,
}

impl fmt::Debug for Overlay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Overlay")
            .field("account_trie_len", &self.account_trie.len())
            .field("storage_trie_len", &self.storage_trie.len())
            .field("account_flat_len", &self.account_flat.len())
            .field("storage_flat_len", &self.storage_flat.len())
            .field("bloom", &"AtomicBloomFilter")
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
        }
    }
}

impl Overlay {
    /// Number of expected items used to size the bloom filter on construction.
    const BLOOM_INITIAL_CAPACITY: usize = 64 * 1024;

    /// Build an overlay by replaying journal entries for blocks
    /// `[from_block, to_block]` (inclusive both) in descending order. Each
    /// loaded entry's `block_hash` is verified against the canonical hash
    /// returned by `expected_hash` for that height; a mismatch aborts and
    /// returns [`OverlayError::HashMismatch`].
    ///
    /// `expected_hash` is a callback that maps a height to the hash of the
    /// canonical block at that height on the chain being unwound. This lets
    /// the caller drive verification from `CANONICAL_BLOCK_HASHES` without
    /// pre-materializing the full chain. Returning `None` from the callback
    /// for a height means "skip verification at this height" (used by tests).
    ///
    /// Within a single key, the OLDEST recorded `prev` value wins (because
    /// later applications overwrite earlier ones in the descending walk):
    /// that's exactly what we want — the value at `to_block - 1` is whatever
    /// the oldest in-range journal entry recorded as the pre-image.
    pub fn from_journal(
        backend: &dyn StorageBackend,
        from_block: BlockNumber,
        to_block: BlockNumber,
        expected_hash: impl Fn(BlockNumber) -> Option<H256>,
    ) -> Result<Self, OverlayError> {
        debug_assert!(from_block >= to_block, "from must be >= to (descending)");
        let mut overlay = Overlay {
            from_block,
            to_block,
            ..Default::default()
        };

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
            overlay.absorb(entry);
            if n == to_block {
                break;
            }
            n -= 1;
        }
        Ok(overlay)
    }

    /// Absorbs one journal entry into the overlay. Later inserts overwrite
    /// earlier ones — combined with a descending walk in `from_journal`, this
    /// makes the OLDEST in-range entry's `prev` win, which is the correct
    /// value at the pivot.
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

    /// Look up `key` in the overlay's `cf` slot.
    ///
    /// Returns:
    /// - `None` if the key is not in the overlay (caller should consult disk).
    /// - `Some(None)` if the key was overwritten and previously didn't exist
    ///   on disk (caller should treat as absent — a rollback would delete it).
    /// - `Some(Some(v))` if the key was overwritten and previously had value
    ///   `v` on disk (caller should treat as `v` — a rollback would restore it).
    #[allow(dead_code, reason = "consumed by the read cascade in Section 7")]
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

    /// Total number of overlay entries across all four CFs. Mostly useful for
    /// tests and metrics.
    #[allow(dead_code, reason = "exposed for tests and Section 14 metrics")]
    pub fn len(&self) -> usize {
        self.account_trie.len()
            + self.storage_trie.len()
            + self.account_flat.len()
            + self.storage_flat.len()
    }

    /// Whether the overlay holds any entries.
    #[allow(dead_code, reason = "exposed for tests and Section 14 metrics")]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Highest block number covered by the overlay (= the cache edge `D` at install time).
    #[allow(clippy::wrong_self_convention, reason = "field accessor: name matches struct field")]
    pub fn from_block(&self) -> BlockNumber {
        self.from_block
    }

    /// Lowest block number covered by the overlay (= `pivot + 1`).
    pub fn to_block(&self) -> BlockNumber {
        self.to_block
    }

    /// Iterates every overlay entry across the four CFs as
    /// `(cf, key, value)` triples. Used by Section 9's reconciliation to fold
    /// overlay-only entries into the first new-chain commit.
    pub fn iter_all_entries(&self) -> impl Iterator<Item = (OverlayCf, &Vec<u8>, &Option<Vec<u8>>)> {
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
    use std::sync::Arc;

    fn h(b: u8) -> H256 {
        H256::repeat_byte(b)
    }

    /// Seed N journal entries directly into STATE_HISTORY. Each entry's
    /// `account_trie_diff` carries one (path, value) pair so we can verify
    /// "older entry wins" semantics across multiple blocks.
    fn seed(
        backend: &Arc<dyn StorageBackend>,
        per_block: &[(BlockNumber, H256, crate::journal::FlatDiff)],
    ) {
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
        let overlay = Overlay::from_journal(backend.as_ref(), 5, 3, |n| {
            Some(H256::repeat_byte(n as u8))
        })
        .unwrap();
        assert_eq!(overlay.len(), 3);
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

    #[test]
    fn older_entry_wins_when_key_repeats() {
        // Block 3 (oldest) wrote K=Y3 (was X). Block 5 (newest) wrote K=Y5
        // (was Y4). The overlay should expose K=X — the value at block 2
        // (= to_block - 1).
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
        // Seed only block 5; ask for [5, 3] — blocks 4 and 3 are missing.
        seed(&backend, &[(5, h(0x05), vec![])]);
        let err = Overlay::from_journal(backend.as_ref(), 5, 3, |_| None).unwrap_err();
        match err {
            OverlayError::MissingEntry(n) => assert_eq!(n, 4),
            other => panic!("expected MissingEntry, got {other:?}"),
        }
    }

    /// Verifies the read cascade precedence on `TrieLayerCache::lookup_overlay`:
    ///
    ///   layer cache (top) ─── covered separately by `TrieLayerCache::get`
    ///   overlay     (mid) ─── this method
    ///   on-disk     (bot) ─── `BackendTrieDB`
    ///
    /// `lookup_overlay` answers only the overlay tier with three possible
    /// outcomes — caller (TrieWrapper::get) uses the result to decide whether
    /// to skip disk.
    #[test]
    fn overlay_lookup_returns_none_when_no_overlay_installed() {
        let cache = TrieLayerCache::new(1);
        // No overlay installed — must short-circuit to None for any key length.
        for key_len in [4usize, 65, 67, 131] {
            let key = vec![0xab; key_len];
            assert_eq!(
                cache.lookup_overlay(&key),
                None,
                "no overlay installed → outer None at length {key_len}"
            );
        }
    }

    #[test]
    fn overlay_lookup_classifies_cf_by_key_length() {
        // Construct an overlay with one entry per CF, each at the canonical
        // length, then assert lookup hits the right bucket.
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        // Build entries directly (bypass from_journal so we control all four CFs).
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

        let mut cache = TrieLayerCache::new(1);
        cache.set_overlay(Arc::new(overlay));

        // Each key must route to its correct CF and produce the right value.
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
        // A different key at the same length must miss the overlay.
        assert_eq!(cache.lookup_overlay(&[0xee; 4]), None);
    }

    #[test]
    fn classify_by_key_length_matches_backend_table_routing() {
        // Spot-check the boundaries: the rules must agree with
        // BackendTrieDB::table_for_key (account leaf at 65, storage leaf at 131,
        // anything else routed by length comparison to 65).
        assert_eq!(OverlayCf::classify_by_key_length(0), OverlayCf::AccountTrie);
        assert_eq!(OverlayCf::classify_by_key_length(64), OverlayCf::AccountTrie);
        assert_eq!(OverlayCf::classify_by_key_length(65), OverlayCf::AccountFlat);
        assert_eq!(OverlayCf::classify_by_key_length(66), OverlayCf::StorageTrie);
        assert_eq!(OverlayCf::classify_by_key_length(130), OverlayCf::StorageTrie);
        assert_eq!(OverlayCf::classify_by_key_length(131), OverlayCf::StorageFlat);
        assert_eq!(OverlayCf::classify_by_key_length(132), OverlayCf::StorageTrie);
    }

    #[test]
    fn skip_verification_when_callback_returns_none() {
        // expected_hash returning None means "don't verify this height";
        // overlay loads regardless of what's on disk.
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        seed(&backend, &[(7, h(0xab), vec![(vec![0x01], None)])]);
        let overlay = Overlay::from_journal(backend.as_ref(), 7, 7, |_| None).unwrap();
        assert_eq!(
            overlay.lookup(OverlayCf::AccountTrie, &[0x01]),
            Some(None)
        );
    }
}
