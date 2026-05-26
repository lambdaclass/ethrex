//! # Trie layering: in-memory diff-layers and deep-reorg overlay
//!
//! This module implements ethrex's two-tier in-memory trie cache that sits
//! between block execution and RocksDB. It is the read/write path for all
//! trie-node and flat-KV accesses during block execution and fork-choice
//! updates.
//!
//! ## Architecture overview
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │  Block N+2  ──► Block N+1  ──► Block N (cache edge D)   │  TrieLayerCache
//! │  (newest layer)              (oldest cached layer)       │  (forward diff-layers)
//! └───────────────────────────┬──────────────────────────────┘
//!                             │ miss
//!                             ▼
//! ┌──────────────────────────────────────────────────────────┐
//! │  Overlay: reverse-diff [D..pivot+1] on the OLD chain     │  (installed only during
//! │  exposes the virtual state at `pivot` without touching   │   deep reorgs; None in
//! │  the on-disk trie.                                       │   steady state)
//! └───────────────────────────┬──────────────────────────────┘
//!                             │ miss (or no overlay)
//!                             ▼
//! ┌──────────────────────────────────────────────────────────┐
//! │  RocksDB on-disk state  (account/storage trie+flat KV)   │
//! └──────────────────────────────────────────────────────────┘
//! ```
//!
//! ## `TrieLayer` — one block's diff
//!
//! Each [`TrieLayer`] stores the trie-node writes produced by executing one
//! block (in regular sync) or one batch of ~1024 blocks (full sync / batch
//! mode). Layers are linked via a `parent` state-root field to form a
//! singly-linked chain from newest to oldest.
//!
//! ## `TrieLayerCache` — the forward cache
//!
//! [`TrieLayerCache`] is a `HashMap<state_root, Arc<TrieLayer>>` with a
//! bloom filter for fast miss detection. When the chain reaches
//! `commit_threshold` layers the oldest eligible layer is flushed to
//! RocksDB and removed from the map. Two thresholds are used:
//! - **128** — regular block-by-block execution.
//! - **4** — full sync / batch mode (one layer ≈ 1 GB of state diffs).
//!
//! ## `Overlay` — the deep-reorg bridge
//!
//! When a fork-choice update targets a head whose ancestor state was flushed
//! past the layer-cache edge `D`, ethrex builds an [`Overlay`] by replaying
//! the [`STATE_HISTORY`](crate::api::tables::STATE_HISTORY) journal entries
//! for blocks `[D, D-1, ..., pivot+1]` in descending order.  The overlay
//! holds the accumulated reverse-diff, exposing the virtual state at `pivot`
//! without mutating RocksDB.
//!
//! ## `TrieWrapper::get` — the read cascade
//!
//! [`TrieWrapper`] is the [`ethrex_trie::TrieDB`] implementation used during
//! block execution. Its `get` method follows a strict priority order:
//!
//! 1. **Layer cache** — forward layers on the new chain (keyed by state-root
//!    chain from the executing block back to the oldest in-memory layer).
//! 2. **Overlay** — if installed, the reverse-diff that reconstructs the
//!    pivot state. A layer hit pre-empts the overlay; an overlay hit pre-empts
//!    disk. `Some(None)` from the overlay means the key was absent at the
//!    pivot (caller must treat as missing, not fall through to disk, because
//!    disk still holds the old chain's value).
//! 3. **Disk** — RocksDB, queried only when both cache and overlay miss.
//!
//! ## Cache swap on deep reorg
//!
//! [`Store::install_overlay_for_reorg`](crate::store::Store::install_overlay_for_reorg)
//! atomically replaces the layer cache with a fresh empty cache that has the
//! newly built overlay pre-installed. Side-chain blocks `[pivot+1 .. new_head]`
//! are then executed via the normal `add_block` path; each block's reads
//! cascade through the overlay and each commit adds a new forward layer.
//! On the first commit the reconciliation step folds the overlay entries and
//! the new layer together into a single atomic RocksDB write batch, then
//! clears the overlay.
//!
//! ## Merged PRs
//!
//! - PR #6686 — initial journal + overlay foundation
//! - PR #6687 — overlay construction from journal
//! - PR #6689 — deep-reorg orchestration (overlay install, side-chain replay)
//! - PR #6685 (tracking) — lift the 128-block reorg cap; this PR

use ethrex_common::{H256, types::BlockNumber};
use fastbloom::AtomicBloomFilter;
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap};
use std::{fmt, sync::Arc};

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
    /// Optional in-memory overlay bridging on-disk state at the cache edge `D` to the
    /// virtual state at a deep-reorg pivot. When installed, reads that miss the layer
    /// chain consult the overlay before falling through to disk. `None` in steady state.
    overlay: Option<Arc<Overlay>>,
}

impl fmt::Debug for TrieLayerCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrieLayerCache")
            .field("last_id", &self.last_id)
            .field("commit_threshold", &self.commit_threshold)
            .field("layers", &self.layers)
            .field("bloom", &"AtomicBloomFilter")
            .field("overlay", &self.overlay)
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
    /// the committed block's identity plus the merged trie node diffs in oldest-first order
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
    /// `parent_state_root` in the returned [`CommitResult`] is the state we'd return to on
    /// rollback (the committed block's pre-state). In normal operation only one layer is
    /// removed; ancestors are evicted as orphans without contributing to the merged nodes
    /// (caught by the `id` retain below).
    pub fn commit(&mut self, state_root: H256) -> Option<CommitResult> {
        let mut layers_to_commit = vec![];
        let mut current_state_root = state_root;
        while let Some(layer) = self.layers.remove(&current_state_root) {
            let layer = Arc::unwrap_or_clone(layer);
            current_state_root = layer.parent;
            layers_to_commit.push(layer);
        }
        // `layers_to_commit` is built by walking parent links from `state_root`,
        // so `.first()` is the newest layer (the one at `state_root` itself).
        //
        // ATTRIBUTION NOTE: In normal block-by-block sync there is only one layer
        // to commit, so attributing the journal entry to `state_root`'s block is
        // correct. If a future caller triggers a multi-layer commit (e.g. by
        // raising the threshold and then forcing a flush), `nodes_to_commit`
        // below would merge diffs from several blocks while the journal entry
        // would still be tagged with only the newest block's identity ; the
        // rollback consumer (PR 2/3) would then be unable to reconstruct
        // intermediate pre-images. Single-layer commits are an invariant of the
        // current write path; revisit this if that ever changes.
        debug_assert!(
            layers_to_commit.len() == 1,
            "multi-layer commit would corrupt journal attribution (see ATTRIBUTION NOTE above): \
             got {} layers, expected 1",
            layers_to_commit.len()
        );
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
///
/// Intentionally not `Default`: an all-zero `CommitResult` would mean a journal entry
/// keyed at block 0 with an empty diff, which is a silent foot-gun (e.g. via
/// `unwrap_or_default()`). Callers must handle the `None` from
/// [`TrieLayerCache::commit`] explicitly.
#[derive(Debug)]
pub struct CommitResult {
    /// Block number of the committed block.
    pub block_number: BlockNumber,
    /// Block hash of the committed block.
    pub block_hash: H256,
    /// Pre-state root of the committed block (the state to return to on rollback).
    pub parent_state_root: H256,
    /// Merged trie node updates in oldest-first order, ready for a sequential disk write.
    pub nodes: Vec<(Vec<u8>, Vec<u8>)>,
}

/// [`TrieDB`] adapter that checks in-memory diff-layers ([`TrieLayerCache`]) first,
/// falling back to the on-disk trie only for keys not found in any layer.
///
/// Used by the EVM during block execution: reads see the latest uncommitted state without
/// waiting for a disk flush.
pub struct TrieWrapper {
    /// State root of the executing block; used as the starting point for the layer-chain walk.
    pub state_root: H256,
    /// Shared reference to the layer cache. Multiple `TrieWrapper` instances (per account/storage
    /// trie) share the same cache within a single block execution context.
    pub inner: Arc<TrieLayerCache>,
    /// The underlying on-disk trie, consulted only when both the layer cache and the overlay miss.
    pub db: Box<dyn TrieDB>,
    /// Pre-computed prefix nibbles for storage tries.
    /// For state tries this is None; for storage tries this is
    /// `Nibbles::from_bytes(address.as_bytes()).append_new(17)`.
    prefix_nibbles: Option<Nibbles>,
}

impl TrieWrapper {
    /// Constructs a `TrieWrapper`. `prefix` is `Some(account_hash)` for storage tries;
    /// pass `None` for the state trie.
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
    /// Non-leaf nodes of the account/state trie (`ACCOUNT_TRIE_NODES` CF, key length < 65).
    AccountTrie,
    /// Non-leaf nodes of storage tries (`STORAGE_TRIE_NODES` CF, key length 66-130).
    StorageTrie,
    /// Leaf entries of the account flat-KV table (`ACCOUNT_FLATKEYVALUE` CF, key length 65).
    AccountFlat,
    /// Leaf entries of the storage flat-KV table (`STORAGE_FLATKEYVALUE` CF, key length 131).
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

    /// Total number of overlay entries across all four CFs.
    pub fn len(&self) -> usize {
        self.account_trie.len()
            + self.storage_trie.len()
            + self.account_flat.len()
            + self.storage_flat.len()
    }

    /// Whether the overlay holds any entries.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Approximate byte size of the overlay's key+value data. O(N) over entries;
    /// intended for one-shot install-time metric emission, NOT per-lookup.
    pub fn byte_size(&self) -> usize {
        [
            &self.account_trie,
            &self.storage_trie,
            &self.account_flat,
            &self.storage_flat,
        ]
        .iter()
        .flat_map(|map| map.iter())
        .map(|(k, v)| k.len() + v.as_ref().map_or(0, |v| v.len()))
        .sum()
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

        let mut cache = TrieLayerCache::new(128);

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
        let cache = TrieLayerCache::new(1);
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

        let mut cache = TrieLayerCache::new(1);
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

        let mut cache = TrieLayerCache::new(1);
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
