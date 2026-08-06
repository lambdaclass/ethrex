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
//! ## Two node sets in one layer
//!
//! On a chain that schedules EIP-8297 (`binaryTreeTime`), a layer carries the
//! same block's **binary-trie** node writes alongside the MPT's, and the cache
//! keeps a secondary index from binary root to layer key so a reader holding
//! only a binary root can find the layer.
//!
//! One layer rather than two parallel caches, because the two node sets must
//! flush atomically — persisting the MPT at block N alongside the binary trie
//! at block N-5 is a recovery problem — and because a single layer gives one
//! reorg mechanism instead of two that have to be kept in step. The secondary
//! index is populated from the root the store *recorded* for the block, never
//! from its header: through the whole pre-activation window a block's header
//! carries an MPT root and its binary root exists only in `BINARY_TRIE_ROOTS`.
//!
//! Binary reads cascade layer chain -> disk through
//! [`LayeredBinaryTrieDB`](crate::binary_trie::LayeredBinaryTrieDB), mirroring
//! [`TrieWrapper`], with one difference the binary trie's storage model forces:
//! an empty value in a layer is a **tombstone**, and answers "absent" without
//! falling through, because the path-keyed single-version node table still
//! holds the node the block removed.
//!
//! **The overlay does not cover binary nodes.** It is reconstructed from
//! `STATE_HISTORY`, whose reverse diffs span the four MPT/flat-KV column
//! families only. Every reorg inside the layer window is covered — those nodes
//! were never written — but a reorg deeper than the cache edge leaves the
//! on-disk binary trie on the abandoned chain.
//! `Store::install_overlay_for_reorg` logs a warning when that happens on a
//! scheduled chain.
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
    binary_trie::BinaryTrieNodes,
    error::StoreError,
    journal::{JournalDecodeError, JournalEntry},
    trie::classify_trie_key,
};

const BLOOM_SIZE: usize = 1_000_000;
const FALSE_POSITIVE_RATE: f64 = 0.02;

#[derive(Debug, Clone)]
struct TrieLayer {
    nodes: FxHashMap<Vec<u8>, Vec<u8>>,
    /// The same block's EIP-8297 binary-trie node writes, keyed by
    /// `BitPath::to_db_key()` — the exact `BINARY_TRIE_NODES` key.
    ///
    /// Held in the *same* layer as `nodes` rather than in a parallel cache, so
    /// the two node sets are staged, flushed and discarded as one unit. They
    /// have to be: persisting the MPT at block N alongside the binary trie at
    /// block N-5 is a recovery problem, and a single layer gets atomicity by
    /// construction instead of by keeping two structures in step.
    ///
    /// Empty on every chain that does not schedule `binaryTreeTime`, which is
    /// the whole cost an unscheduled chain pays for this.
    ///
    /// An empty *value* is a tombstone, matching `BinaryTrieDB`'s convention:
    /// the node left the tree, so a reader must answer "absent" and must not
    /// fall through to the stale node still on disk at that path.
    binary_nodes: FxHashMap<Vec<u8>, Vec<u8>>,
    /// Binary-trie root this layer's `binary_nodes` produce, i.e. the root the
    /// block's binary post-state is addressed by. `H256::zero()` when the chain
    /// is not binary-tree-scheduled and this layer holds no binary nodes.
    ///
    /// Layers are keyed by *header* state root, which through the whole
    /// pre-activation window is the MPT root — a block's binary root is not in
    /// its header until activation. So the binary root cannot be recovered from
    /// the key and is recorded here, feeding
    /// [`TrieLayerCache::binary_index`](TrieLayerCache).
    binary_root: H256,
    parent: H256,
    id: usize,
    /// Number of the block whose post-state this layer represents. Used by the
    /// journal write path so a commit can record the entry under the correct
    /// block number (not the in-flight block whose insertion triggered the commit).
    block_number: BlockNumber,
    /// Hash of the block whose post-state this layer represents.
    block_hash: H256,
}

/// One block's binary-trie contribution to a diff layer: the root its writes
/// produce, and the writes themselves as `BINARY_TRIE_NODES` key/value pairs
/// (an empty value being a tombstone).
///
/// [`Default`] is the "this chain does not schedule the commitment" case: a
/// zero root and no nodes, which stages nothing and indexes nothing.
#[derive(Debug, Clone, Default)]
pub struct BinaryLayerUpdate {
    /// Binary-trie root after this block's writes. `H256::zero()` means "no
    /// binary state for this block".
    pub root: H256,
    /// `(BINARY_TRIE_NODES key, encoded node)` pairs; empty value = tombstone.
    pub nodes: BinaryTrieNodes,
}

impl BinaryLayerUpdate {
    /// Whether this update carries binary state at all. A zero root means the
    /// chain does not schedule the commitment; nodes without a root would be
    /// unaddressable, so both are required.
    fn is_present(&self) -> bool {
        !self.root.is_zero()
    }
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
    /// Secondary index over the same layers: binary-trie root -> the layer's
    /// key (its block's header state root).
    ///
    /// It exists because layers are keyed by header state root and, before
    /// activation, a block's binary root is *not* in its header — it is only
    /// the root the store recorded for it. So the index is populated from that
    /// recorded root ([`BinaryLayerUpdate::root`]) at insertion time, never
    /// derived from the header. Post-activation the two roots coincide and the
    /// index maps a root to itself, which costs one entry and keeps one code
    /// path for both regimes.
    ///
    /// Rebuilt from the surviving layers whenever [`Self::commit`] prunes, so a
    /// discarded branch's binary root stops resolving at exactly the moment its
    /// nodes are dropped.
    binary_index: FxHashMap<H256, H256>,
    /// Bloom filter over every binary-trie key in every layer, the binary
    /// counterpart of `bloom`.
    ///
    /// Separate rather than shared so that an unscheduled chain's MPT lookups
    /// keep exactly the false-positive rate they had: binary keys are 34 bytes
    /// and would otherwise crowd the same filter.
    binary_bloom: AtomicBloomFilter<FxBuildHasher>,
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
            .field("binary_index_len", &self.binary_index.len())
            .field("overlay", &self.overlay)
            .field("safe_commit_root", &safe_commit)
            .finish()
    }
}

impl Default for TrieLayerCache {
    fn default() -> Self {
        Self {
            bloom: Self::create_filter(BLOOM_SIZE),
            binary_index: Default::default(),
            binary_bloom: Self::create_filter(BLOOM_SIZE),
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
            binary_index: Default::default(),
            binary_bloom: Self::create_filter(BLOOM_SIZE),
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

    /// Looks up an EIP-8297 binary-trie node `key` (a `BINARY_TRIE_NODES` key)
    /// as of `binary_root`, walking the same layer chain the MPT walks.
    ///
    /// Three-state return, because the binary trie's tombstone convention makes
    /// "absent" load-bearing:
    /// - `None` — no layer on this chain wrote the key. The caller falls
    ///   through to disk.
    /// - `Some(None)` — a layer tombstoned it. The node left the tree, so the
    ///   caller must answer "absent" and must **not** consult disk, which is
    ///   path-keyed and single-version and still holds the superseded node.
    /// - `Some(Some(v))` — the newest layer that wrote it holds `v`.
    ///
    /// `binary_root` resolves through [`Self::binary_index`](Self) to the
    /// layer's own key, after which the walk follows `parent` exactly as
    /// [`Self::get`] does: the binary chain and the MPT chain are the same
    /// chain of blocks, so one set of parent links serves both. Layers with no
    /// binary nodes (an unscheduled stretch) simply miss and the walk goes on.
    pub fn binary_get(&self, binary_root: H256, key: &[u8]) -> Option<Option<Vec<u8>>> {
        // Same fast path as `get`: a bloom miss is proof no layer has the key.
        if !self.binary_bloom.contains(key) {
            return None;
        }
        let mut current = *self.binary_index.get(&binary_root)?;
        let mut steps = 0usize;
        // Bounded walk, matching `get_commitable`: a malformed parent link must
        // terminate rather than spin, and unlike `get` this path has no
        // "state cycle" invariant to lean on (the index is a second entry point
        // into the same links).
        let max_steps = self.layers.len();
        while let Some(layer) = self.layers.get(&current) {
            if let Some(value) = layer.binary_nodes.get(key) {
                return Some((!value.is_empty()).then(|| value.clone()));
            }
            current = layer.parent;
            steps += 1;
            if steps > max_steps {
                return None;
            }
        }
        None
    }

    /// Whether `binary_root` has a diff layer holding its binary state. The
    /// binary counterpart of [`Self::has_layer`].
    pub fn has_binary_layer(&self, binary_root: H256) -> bool {
        self.binary_index.contains_key(&binary_root)
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
        // (c) The executed parent IS the safe-commit root. Still requires a layer: a
        // canonical root need not have one (`put_batch` skips blocks whose state root equals
        // their parent's, which on L2 is every empty block). Branch (d) gets this for free
        // by walking `layers`.
        if parent_state_root == safe_root {
            return self.has_layer(safe_root).then_some(safe_root);
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

    /// Depth-only commit gate for single-canonical-chain re-execution (full sync, block
    /// import, startup state regeneration).
    ///
    /// Walks the parent chain from `state_root`, counting layers, and returns the state root of
    /// the layer that is `threshold` layers deep — committing purely by depth, ignoring the
    /// canonical [`safe_commit_root`](Self::safe_commit_root) cell.
    ///
    /// Used only where the node extends a single canonical chain (these paths never execute
    /// competing forks), so the non-canonical-commit hazard that the canonical gate guards
    /// against cannot occur. The canonical gate keys on the `head - 128` safe-commit root, which
    /// these paths never advance (nothing is canonicalized until a later forkchoice update), so
    /// it would never flush during re-execution; this depth gate bounds memory instead.
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
        self.put_batch_with_binary(
            parent,
            state_root,
            block_number,
            block_hash,
            key_values,
            BinaryLayerUpdate::default(),
        )
    }

    /// [`Self::put_batch`] carrying the same block's EIP-8297 binary-trie
    /// writes into the same layer.
    ///
    /// One layer, two node sets: they are staged together, flushed together by
    /// [`Self::commit`] and discarded together when a branch is abandoned. The
    /// binary root is additionally recorded in
    /// [`binary_index`](Self::binary_get) so a reader holding only a binary
    /// root can find the layer — necessary because before activation a block's
    /// binary root is nowhere in its header.
    ///
    /// The empty-block guard below is shared: an MPT root that does not move
    /// means the state did not change, which means the binary root did not move
    /// either, so there is nothing to stage on either side.
    pub fn put_batch_with_binary(
        &mut self,
        parent: H256,
        state_root: H256,
        block_number: BlockNumber,
        block_hash: H256,
        key_values: Vec<(Nibbles, Vec<u8>)>,
        binary: BinaryLayerUpdate,
    ) {
        if parent == state_root && key_values.is_empty() && binary.nodes.is_empty() {
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
        for (p, _) in &binary.nodes {
            self.binary_bloom.insert(p);
        }

        let nodes: FxHashMap<Vec<u8>, Vec<u8>> = key_values
            .into_iter()
            .map(|(path, value)| (path.into_vec(), value))
            .collect();
        let binary_present = binary.is_present();
        let binary_root = binary.root;
        let binary_nodes: FxHashMap<Vec<u8>, Vec<u8>> = binary.nodes.into_iter().collect();

        self.last_id += 1;
        let entry = TrieLayer {
            nodes,
            binary_nodes,
            binary_root,
            parent,
            id: self.last_id,
            block_number,
            block_hash,
        };
        self.layers.insert(state_root, Arc::new(entry));
        if binary_present {
            self.binary_index.insert(binary_root, state_root);
        }
    }

    /// Rebuilds the global bloom filter from scratch using all keys across all remaining layers.
    ///
    /// Called after [`commit`](Self::commit) removes layers, since the old filter may contain
    /// keys from the removed layers (producing unnecessary false positives).
    ///
    /// Rebuilds the binary-trie filter and the binary-root index in the same
    /// pass, for the same reason and with a stronger requirement: a stale
    /// `binary_index` entry would let an abandoned branch's root resolve to a
    /// layer that is no longer its own, so the index must be derived from the
    /// surviving layers rather than patched.
    pub fn rebuild_bloom(&mut self) {
        // Pre-compute total keys for optimal filter sizing
        let total_keys: usize = self.layers.values().map(|layer| layer.nodes.len()).sum();
        let total_binary_keys: usize = self
            .layers
            .values()
            .map(|layer| layer.binary_nodes.len())
            .sum();

        let filter = Self::create_filter(total_keys.max(BLOOM_SIZE));
        let binary_filter = Self::create_filter(total_binary_keys.max(BLOOM_SIZE));

        // Parallel insertion - AtomicBloomFilter allows concurrent insert via &self
        self.layers.par_iter().for_each(|(_, layer)| {
            for path in layer.nodes.keys() {
                filter.insert(path);
            }
            for path in layer.binary_nodes.keys() {
                binary_filter.insert(path);
            }
        });

        self.bloom = filter;
        self.binary_bloom = binary_filter;
        self.binary_index = self
            .layers
            .iter()
            .filter(|(_, layer)| !layer.binary_root.is_zero())
            .map(|(state_root, layer)| (layer.binary_root, *state_root))
            .collect();
    }

    /// Whether `state_root` has a diff layer. Not every canonical root does: [`Self::put_batch`]
    /// skips blocks whose state root equals their parent's, and flushed roots are pruned. Callers
    /// holding a root from outside the cache must check this before treating it as committable.
    pub fn has_layer(&self, state_root: H256) -> bool {
        self.layers.contains_key(&state_root)
    }

    /// Number of diff layers held in memory. Diagnostic only: distinguishes a pruned root from
    /// an empty cache.
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// Returns the root of the oldest layer in the layer chain containing `root` — i.e.
    /// walks parents until reaching a layer whose parent is not itself a layer. Returns
    /// `root` unchanged if it has no layer.
    ///
    /// Used to force single-layer commits while a deep-reorg overlay is installed: the
    /// The reconciliation in `commit_to_disk` is defined for exactly one layer at
    /// the pivot tip `T`, and a multi-layer sweep would journal upper layers' pre-images
    /// against the old-chain disk state instead of the new-chain/bridge state.
    pub(crate) fn bottom_layer_root(&self, root: H256) -> H256 {
        let mut current = root;
        while let Some(layer) = self.layers.get(&current) {
            if !self.layers.contains_key(&layer.parent) {
                break;
            }
            current = layer.parent;
        }
        current
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
                binary_nodes: layer.binary_nodes.into_iter().collect(),
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
    /// Block number of the committed block.
    pub block_number: BlockNumber,
    /// Block hash of the committed block.
    pub block_hash: H256,
    /// Pre-state root of the committed block (the state to return to on rollback).
    pub parent_state_root: H256,
    /// Merged trie node updates in oldest-first order, ready for a sequential disk write.
    pub nodes: Vec<(Vec<u8>, Vec<u8>)>,
    /// The same block's EIP-8297 binary-trie node updates, as
    /// `BINARY_TRIE_NODES` key/value pairs with an empty value meaning
    /// "delete this key". Empty on unscheduled chains.
    ///
    /// Written into the same `write_tx` as `nodes`, which is the point: the two
    /// tries advance on disk in one atomic step, never independently.
    pub binary_nodes: BinaryTrieNodes,
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
        // While a deep-reorg overlay serves this root, flat-KV leaf reads must not
        // trust disk: journal entries written while the FKV generator was running
        // are permanently missing pre-images for keys past the generator frontier,
        // and disk flat-KV may hold the generator's value for the chain
        // being reorged away. Force the trie-node read path instead — trie nodes
        // are always journaled, so the overlay reconstructs them completely.
        if self.inner.overlay_serves(self.state_root) {
            return false;
        }
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
/// atomic write (the reconciliation step in `commit_to_disk`).
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
        // could cause a spurious `MissingEntry`. This is prevented by the
        // reorg-in-progress guard: `Blockchain::enter_reorg` holds
        // `Store::set_journal_pruning_paused(true)` for the whole apply pass, so
        // pruning is deferred while the overlay is constructed (it catches up on
        // the next finality advance after the pass).
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
    /// by the reconciliation step in `commit_to_disk` to fold overlay-only entries
    /// into the first new-chain commit.
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
    /// for the review finding "Overlay Applies Across Roots".
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

    /// While an overlay serves the read's state root, `flatkeyvalue_computed`
    /// must return false so `Trie::get` walks the (always journaled) trie nodes
    /// instead of trusting disk flat-KV, which may hold the generator's stale,
    /// unjournaled values. Roots the overlay does not serve must be unaffected.
    #[test]
    fn flatkeyvalue_computed_is_disabled_while_overlay_serves() {
        struct AlwaysComputedDb;
        impl TrieDB for AlwaysComputedDb {
            fn flatkeyvalue_computed(&self, _key: Nibbles) -> bool {
                true
            }
            fn get(&self, _key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
                Ok(None)
            }
            fn put_batch(&self, _key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
                unimplemented!()
            }
        }

        let pivot = h(0xaa);
        let unrelated = h(0xcc);
        let key = Nibbles::from_bytes(&[0x01; 32]);

        let mut cache =
            TrieLayerCache::new_with_safe_commit(128, Arc::new(RwLock::new(H256::zero())));
        cache.set_overlay(Arc::new(Overlay {
            serves_root: pivot,
            ..Default::default()
        }));
        let cache = Arc::new(cache);

        let served = TrieWrapper::new(pivot, cache.clone(), Box::new(AlwaysComputedDb), None);
        assert!(
            !served.flatkeyvalue_computed(key.clone()),
            "served root must not trust disk flat-KV"
        );

        let unserved = TrieWrapper::new(unrelated, cache.clone(), Box::new(AlwaysComputedDb), None);
        assert!(
            unserved.flatkeyvalue_computed(key.clone()),
            "unserved root must keep the flat-KV fast path"
        );

        // Same for storage tries (prefixed wrapper).
        let served_storage =
            TrieWrapper::new(pivot, cache, Box::new(AlwaysComputedDb), Some(h(0x01)));
        assert!(
            !served_storage.flatkeyvalue_computed(key),
            "served storage root must not trust disk flat-KV"
        );
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
        // Sanity check for the reconciliation path: every CF an entry was inserted
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

    /// (c2) Regression: `parent_state_root == safe_root` but that root has no layer, the steady
    /// state on L2 where empty blocks keep their parent's root. Branch (c) used to match on the
    /// root value alone and hand `commit_to_disk` something it could not commit.
    #[test]
    fn parent_equals_safe_root_without_layer_yields_none() {
        let orphan = h256(7);
        let (mut cache, _cell) = cache_with_cell(4, orphan);
        // An empty block: parent == state_root, so `put_batch` inserts nothing.
        cache.put_batch(orphan, orphan, 7, orphan, vec![]);
        assert!(
            !cache.has_layer(orphan),
            "empty-block root must not create a layer"
        );
        assert_eq!(
            cache.get_commitable(orphan),
            None,
            "a safe root with no layer is not committable, even when it equals the parent"
        );
    }

    /// (c3) A flushed root is pruned by `commit`, so it must not be offered again.
    #[test]
    fn already_committed_safe_root_yields_none() {
        let safe = h256(2);
        let (mut cache, _cell) = cache_with_cell(4, safe);
        build_chain(&mut cache, 3);
        assert_eq!(cache.get_commitable(safe), Some(safe));
        cache.commit(safe).expect("first commit flushes the layer");
        assert!(!cache.has_layer(safe), "commit prunes the flushed layer");
        assert_eq!(
            cache.get_commitable(safe),
            None,
            "re-committing an already-flushed root must not be offered again"
        );
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

    // -----------------------------------------------------------------------
    // Binary-trie node sets, secondary index and tombstones.
    // -----------------------------------------------------------------------

    use super::BinaryLayerUpdate;

    /// A binary-trie node key of the shape `BitPath::to_db_key` produces.
    fn bkey(b: u8) -> Vec<u8> {
        let mut key = vec![0u8; 34];
        key[0] = b;
        key
    }

    /// One block's layer carrying both node sets. `n` names the MPT root, and
    /// the binary root is deliberately a *different* value (`0x80 + n`) so no
    /// test can pass by accidentally treating the two as interchangeable —
    /// which is exactly the pre-activation regime.
    fn put_dual(cache: &mut TrieLayerCache, parent: H256, n: u8, binary: Vec<(Vec<u8>, Vec<u8>)>) {
        cache.put_batch_with_binary(
            parent,
            h256(n),
            n as u64,
            h256(n),
            vec![(key(n), vec![n])],
            BinaryLayerUpdate {
                root: h256(0x80 + n),
                nodes: binary,
            },
        );
    }

    /// The secondary index is what makes a binary root addressable at all: the
    /// layer is keyed by the MPT root, and before activation nothing in the
    /// block's header names its binary root.
    #[test]
    fn a_binary_root_resolves_through_the_secondary_index() {
        let (mut cache, _cell) = cache_with_cell(4, H256::zero());
        put_dual(&mut cache, H256::zero(), 1, vec![(bkey(0xa), vec![0x11])]);

        assert!(cache.has_binary_layer(h256(0x81)));
        assert_eq!(
            cache.binary_get(h256(0x81), &bkey(0xa)),
            Some(Some(vec![0x11]))
        );
        // The MPT root is not a binary root: asking with the layer's own key
        // must find nothing, or the two indexes have been conflated.
        assert_eq!(cache.binary_get(h256(1), &bkey(0xa)), None);
        // An unknown binary root falls through to disk.
        assert_eq!(cache.binary_get(h256(0xff), &bkey(0xa)), None);
    }

    /// The walk follows the MPT parent links — one chain of blocks serves both
    /// node sets — and the newest layer that wrote a key wins.
    #[test]
    fn the_binary_walk_follows_the_layer_parent_chain() {
        let (mut cache, _cell) = cache_with_cell(4, H256::zero());
        put_dual(
            &mut cache,
            H256::zero(),
            1,
            vec![(bkey(0xa), vec![0x11]), (bkey(0xb), vec![0xbb])],
        );
        put_dual(&mut cache, h256(1), 2, vec![(bkey(0xa), vec![0x22])]);
        put_dual(&mut cache, h256(2), 3, vec![(bkey(0xc), vec![0xcc])]);

        let tip = h256(0x83);
        assert_eq!(
            cache.binary_get(tip, &bkey(0xa)),
            Some(Some(vec![0x22])),
            "the newest layer that wrote the key must win"
        );
        assert_eq!(
            cache.binary_get(tip, &bkey(0xb)),
            Some(Some(vec![0xbb])),
            "a key only the oldest layer wrote must still be found"
        );
        // Reading at an older root must not see a newer layer's write.
        assert_eq!(
            cache.binary_get(h256(0x81), &bkey(0xa)),
            Some(Some(vec![0x11]))
        );
        assert_eq!(cache.binary_get(h256(0x81), &bkey(0xc)), None);
    }

    /// The tombstone rule, which the MPT does not need: an empty value means
    /// the node left the tree, and the reader must answer "absent" rather than
    /// fall through to the superseded node still on the path-keyed disk.
    #[test]
    fn an_empty_binary_value_reads_as_absent_and_does_not_fall_through() {
        let (mut cache, _cell) = cache_with_cell(4, H256::zero());
        put_dual(&mut cache, H256::zero(), 1, vec![(bkey(0xa), vec![0x11])]);
        put_dual(&mut cache, h256(1), 2, vec![(bkey(0xa), vec![])]);

        assert_eq!(
            cache.binary_get(h256(0x82), &bkey(0xa)),
            Some(None),
            "Some(None) is 'deleted here'; a bare None would send the caller to disk"
        );
        // The layer below still holds the value at its own root.
        assert_eq!(
            cache.binary_get(h256(0x81), &bkey(0xa)),
            Some(Some(vec![0x11]))
        );
    }

    /// Committing removes both node sets from memory together and stops the
    /// committed binary root resolving — its nodes are on disk now.
    #[test]
    fn commit_hands_back_both_node_sets_and_drops_the_index_entry() {
        let safe = h256(1);
        let (mut cache, _cell) = cache_with_cell(4, safe);
        put_dual(&mut cache, H256::zero(), 1, vec![(bkey(0xa), vec![0x11])]);
        put_dual(&mut cache, h256(1), 2, vec![(bkey(0xb), vec![0x22])]);

        let committed = cache.commit(safe).expect("layer 1 is committable");
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].binary_nodes, vec![(bkey(0xa), vec![0x11])]);
        assert!(
            !committed[0].nodes.is_empty(),
            "the same CommittedLayer must carry the MPT nodes: one flush, not two"
        );

        assert!(!cache.has_binary_layer(h256(0x81)));
        assert_eq!(cache.binary_get(h256(0x81), &bkey(0xa)), None);
        // The layer above survives, index entry intact.
        assert!(cache.has_binary_layer(h256(0x82)));
        assert_eq!(
            cache.binary_get(h256(0x82), &bkey(0xb)),
            Some(Some(vec![0x22]))
        );
    }

    /// A reorg: two branches off one fork point, the canonical one extended and
    /// then committed. `commit` prunes every layer older than the one it
    /// flushed, which is what discards the abandoned branch — and because that
    /// branch's binary nodes were only ever staged, they are dropped without
    /// ever being handed to the disk writer.
    ///
    /// Both branches write the *same* path (`0xff`) with different values, the
    /// collision the single-version on-disk table cannot represent and that
    /// made write-through unsafe.
    #[test]
    fn committing_one_branch_discards_the_abandoned_branchs_binary_nodes() {
        let base = h256(1);
        let (mut cache, _cell) = cache_with_cell(4, H256::zero());
        put_dual(&mut cache, H256::zero(), 1, vec![(bkey(0x01), vec![0x01])]);
        // The branch that loses, imported first.
        put_dual(&mut cache, base, 9, vec![(bkey(0xff), vec![0x99])]);
        // The branch that wins, imported after it and then extended, so the
        // commit at its first block sits above the abandoned layer's id.
        put_dual(&mut cache, base, 2, vec![(bkey(0xff), vec![0x22])]);
        put_dual(&mut cache, h256(2), 3, vec![(bkey(0x03), vec![0x33])]);

        assert!(
            cache.has_binary_layer(h256(0x89)),
            "the losing branch is resident before the commit"
        );

        let committed = cache.commit(h256(2)).expect("the winner's first block");
        let flushed: Vec<(&Vec<u8>, &Vec<u8>)> = committed
            .iter()
            .flat_map(|layer| layer.binary_nodes.iter().map(|(k, v)| (k, v)))
            .collect();
        assert_eq!(
            flushed,
            vec![(&bkey(0x01), &vec![0x01]), (&bkey(0xff), &vec![0x22])],
            "only the fork point and the winning branch may be flushed, and the shared \
             path must carry the winner's value"
        );
        assert!(
            !cache.has_binary_layer(h256(0x89)),
            "the abandoned branch's binary root must stop resolving"
        );
        assert_eq!(cache.binary_get(h256(0x89), &bkey(0xff)), None);
        assert_eq!(
            cache.binary_get(h256(0x83), &bkey(0x03)),
            Some(Some(vec![0x33])),
            "the surviving branch's still-resident layer is untouched"
        );
    }

    /// An unscheduled chain's layers carry no binary state at all: no index
    /// entry, no nodes handed to the flush, nothing.
    #[test]
    fn an_unscheduled_layer_carries_no_binary_state() {
        let safe = h256(1);
        let (mut cache, _cell) = cache_with_cell(4, safe);
        build_chain(&mut cache, 2);

        assert!(!cache.has_binary_layer(h256(1)));
        assert!(!cache.has_binary_layer(H256::zero()));
        assert_eq!(cache.binary_get(h256(1), &bkey(0xa)), None);

        let committed = cache.commit(safe).expect("committable");
        assert!(
            committed.iter().all(|layer| layer.binary_nodes.is_empty()),
            "an unscheduled chain must flush no binary nodes"
        );
    }
}
