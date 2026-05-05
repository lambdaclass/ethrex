//! Binary-trie-specific wiring: trie provider, disk commits, cache layer.
//!
//! This module is the binary-trie parallel to `mpt_wiring.rs`. Every production
//! primitive that MPT has, binary must have an equivalent (or a documented stub
//! where the MPT primitive is not applicable — e.g. there is no binary FKV
//! background generator because binary trie starts empty post-switch and grows
//! only from commits; snap sync stays MPT-only).
//!
//! # Key-space design
//!
//! `BINARY_TRIE_NODES` houses three distinct key ranges:
//! - `[u64 LE; 8 bytes]` — serialized node data (InternalNode or StemNode).
//! - `[0xFF, ...]` prefix — metadata (META_ROOT, META_NEXT_ID, etc.).
//! - `[0xFE, stem...; 32 bytes]` — tombstone marker for a SELFDESTRUCTed stem.
//!   The `0xFE` prefix is disjoint from both 8-byte NodeId LE keys and the
//!   `0xFF` meta-key range, so there are no collisions.
//!
//! # FKV generation model
//!
//! Unlike MPT, binary trie has no background FKV generator. `BINARY_FLATKEYVALUE`
//! is populated inline by `binary_commit_nodes_to_disk` in the same write
//! transaction as the trie nodes. This is safe because binary trie starts empty
//! post-switch and every leaf is written through a commit — there is no
//! "load snap-synced nodes and denormalize" scenario (snap sync stays MPT-only).

use crate::{
    Store,
    api::{
        StorageBackend,
        tables::{BINARY_FLATKEYVALUE, BINARY_STORAGE_KEYS, BINARY_TRIE_NODES},
    },
    error::StoreError,
    state_backend::StateBackend,
};
use ethrex_binary_trie::{
    BinaryBackend, BinaryTrieError, BinaryTrieProvider,
    db::{TrieBackend, WriteOp},
    hash::{CACHE_TOMBSTONE_TAG, CACHE_VALUE_TAG},
    node_store::META_ROOT_HASH,
    state::BinaryTrieState,
};
use ethrex_common::H256;
use ethrex_state_backend::CodeReader;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Tombstone key helpers
// ---------------------------------------------------------------------------

/// Build the `BINARY_TRIE_NODES` key for a stem tombstone.
///
/// Key format: `[0xFE, stem[0], stem[1], ..., stem[30]]` = 32 bytes total.
///
/// Tombstone keys are disjoint from other key ranges by **length**, not by
/// first-byte value range:
/// - 8-byte NodeId LE keys: exactly 8 bytes (no prefix, raw u64 LE).
/// - Metadata keys (`META_ROOT`, `META_NEXT_ID`, `META_ROOT_HASH`, etc.):
///   `0xFF`-prefixed, 2-3 bytes total.
/// - Tombstone keys: exactly 32 bytes, starting with `0xFE`.
///
/// All three ranges have distinct fixed lengths (8, 2-3, 32), so there are no
/// collisions regardless of the value of the first byte.
fn tombstone_key(stem: &[u8; 31]) -> [u8; 32] {
    let mut key = [0u8; 32];
    key[0] = 0xFE;
    key[1..32].copy_from_slice(stem);
    key
}

// ---------------------------------------------------------------------------
// StorageTrieBackend — TrieBackend adapter over StorageBackend
// ---------------------------------------------------------------------------

/// Adapts a cloned `Store` (or any `StorageBackend`) into the `TrieBackend`
/// trait required by `NodeStore::open` and `BinaryTrieState::open`.
///
/// This lets us construct a DB-backed `NodeStore` from the storage layer so that
/// `new_binary_state_reader` can open a trie that lazily loads nodes from disk.
pub(crate) struct StorageTrieBackend {
    pub(crate) store: Store,
}

impl TrieBackend for StorageTrieBackend {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        let tx = self
            .store
            .backend
            .begin_read()
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        tx.get(table, key)
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))
    }

    fn write_batch(&self, ops: Vec<WriteOp>) -> Result<(), BinaryTrieError> {
        let mut tx = self
            .store
            .backend
            .begin_write()
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        for op in ops {
            match op {
                WriteOp::Put { table, key, value } => tx
                    .put(table, &key, &value)
                    .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?,
                WriteOp::Delete { table, key } => tx
                    .delete(table, &key)
                    .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?,
            }
        }
        tx.commit()
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))
    }

    fn full_iterator(
        &self,
        table: &'static str,
    ) -> Result<Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)>>, BinaryTrieError> {
        let tx = self
            .store
            .backend
            .begin_read()
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        // Prefix with empty slice = full table scan.
        let iter = tx
            .prefix_iterator(table, &[])
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        let entries: Vec<(Vec<u8>, Vec<u8>)> = iter
            .filter_map(|r| r.ok().map(|(k, v)| (k.into_vec(), v.into_vec())))
            .collect();
        Ok(Box::new(entries.into_iter()))
    }
}

// ---------------------------------------------------------------------------
// CacheAwareTrieBackend — TrieBackend that consults trie_cache at the live
// binary head root before falling through to disk.
// ---------------------------------------------------------------------------

/// `TrieBackend` wrapper that consults the in-memory `trie_cache` at the
/// current binary head root for reads on `BINARY_TRIE_NODES` before falling
/// through to the disk-backed [`StorageTrieBackend`].
///
/// Used by [`Store::new_transition_state_reader`] so the in-memory
/// [`BinaryTrieState`] traverses the **live** trie structure (cache layers +
/// disk), not the disk-flushed root which lags by up to 128 layers. Symmetric
/// to MPT's `MptTrieWrapper(state_root, trie_cache, db, last_written)`.
///
/// Other tables (`BINARY_STORAGE_KEYS`, etc.) bypass the cache and go straight
/// to disk.
pub(crate) struct CacheAwareTrieBackend {
    pub(crate) store: Store,
    pub(crate) inner: StorageTrieBackend,
}

impl TrieBackend for CacheAwareTrieBackend {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        if table == BINARY_TRIE_NODES {
            let head = self.store.current_binary_root();
            let cache = self
                .store
                .trie_cache
                .read()
                .map_err(|_| BinaryTrieError::StoreError("trie_cache RwLock poisoned".into()))?
                .clone();
            if let Some(framed) = cache.get(head, key) {
                // Cache hit: decode framing.
                // - Ok(Some(bytes)) -> value-tagged: return the unframed bytes.
                // - Ok(None) -> tombstone-tagged: return None (treat as absent;
                //   either a deleted node or a stem-tombstone marker).
                return decode_binary_cache_value(&framed)
                    .map_err(|e| BinaryTrieError::StoreError(e.to_string()));
            }
        }
        self.inner.get(table, key)
    }

    fn write_batch(&self, ops: Vec<WriteOp>) -> Result<(), BinaryTrieError> {
        self.inner.write_batch(ops)
    }

    fn full_iterator(
        &self,
        table: &'static str,
    ) -> Result<Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)>>, BinaryTrieError> {
        self.inner.full_iterator(table)
    }
}

// ---------------------------------------------------------------------------
// StoreBinaryTrieProvider — BinaryTrieProvider backed by Store
// ---------------------------------------------------------------------------

/// [`BinaryTrieProvider`] implementation backed by a [`Store`].
///
/// Created by [`Store::make_binary_trie_provider`] and passed to
/// `BinaryMerkleizer::new` and `BinaryBackend::new_with_db`.
pub(crate) struct StoreBinaryTrieProvider {
    pub(crate) store: Store,
}

impl BinaryTrieProvider for StoreBinaryTrieProvider {
    /// Load a serialized trie node by its 8-byte node ID (little-endian u64).
    fn load_node(&self, id: u64) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        let key = id.to_le_bytes();
        let tx = self
            .store
            .backend
            .begin_read()
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        tx.get(BINARY_TRIE_NODES, &key)
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))
    }

    /// Load a metadata entry by raw key (e.g. `META_ROOT`, `META_NEXT_ID`, or
    /// any `0xFF`-prefixed custom key stored alongside node data).
    fn load_meta(&self, key: &[u8]) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        let tx = self
            .store
            .backend
            .begin_read()
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        tx.get(BINARY_TRIE_NODES, key)
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))
    }

    /// Returns `true` if the given 31-byte stem has a tombstone entry in the
    /// in-memory binary cache (live head root) or on disk.
    ///
    /// In-memory check is essential during overlay: a SELFDESTRUCT in block N
    /// writes the tombstone into `trie_cache` (Phase 1) but only flushes to
    /// disk once 128 layers accumulate (Phase 2). Block N+1 reads must see the
    /// tombstone immediately, otherwise the transition fall-through resurrects
    /// the storage of the destroyed account from MPT base. Symmetric to the
    /// `cache_get_leaf` walk that fixed Bug 4 for value reads.
    fn is_deleted_stem(&self, stem: &[u8; 31]) -> Result<bool, BinaryTrieError> {
        let key = tombstone_key(stem);

        // In-memory cache walk: tombstones are framed as [CACHE_TOMBSTONE_TAG]
        // in the binary node layer, keyed by [0xFE, stem...]. Walk the layer
        // chain from current_binary_root toward older ancestors.
        let head = self.store.current_binary_root();
        let cache = self
            .store
            .trie_cache
            .read()
            .map_err(|_| BinaryTrieError::StoreError("trie_cache RwLock poisoned".into()))?
            .clone();
        if let Some(framed) = cache.get(head, &key) {
            // Any framed entry at a tombstone key is a tombstone (the only
            // writes at [0xFE, stem] keys are tombstone tag values).
            // Decode defensively in case framing changes; either tombstone tag
            // or non-empty value at this key means the stem is deleted.
            return match decode_binary_cache_value(&framed)
                .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?
            {
                None => Ok(true),    // tombstone tag: deleted
                Some(_) => Ok(true), // any value at a 0xFE key means presence-marker
            };
        }

        // Fall through to disk.
        let tx = self
            .store
            .backend
            .begin_read()
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        let found = tx
            .get(BINARY_TRIE_NODES, &key)
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        Ok(found.is_some())
    }

    /// Returns `true` if the given 32-byte tree key has any FKV row in the
    /// `BINARY_FLATKEYVALUE` table on disk — including an explicit `[0; 32]`
    /// marker from a post-switch SSTORE 0.
    ///
    /// This only checks disk. The in-memory layer cache is checked separately
    /// via `cache_get_leaf`; both `BinaryBackend::slot_is_in_overlay` and the
    /// transition read path consult the cache first.
    fn is_slot_in_fkv(&self, tree_key: &[u8; 32]) -> Result<bool, BinaryTrieError> {
        let tx = self
            .store
            .backend
            .begin_read()
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        let found = tx
            .get(BINARY_FLATKEYVALUE, tree_key)
            .map_err(|e| BinaryTrieError::StoreError(e.to_string()))?;
        Ok(found.is_some())
    }

    /// Walk the in-memory `binary_trie_cache` from the live head root
    /// (`Store::current_binary_root`) toward older ancestors and return the
    /// first layer's record for `tree_key`, if any.
    ///
    /// Returns:
    /// - `Some(Some(value))` — leaf was written with this value in some layer
    /// - `Some(None)` — leaf was explicitly deleted (SSTORE 0 / SELFDESTRUCT
    ///   stem clear) in some layer
    /// - `None` — leaf is not in any cache layer; caller must fall through
    ///   to disk (`BINARY_FLATKEYVALUE`) or to the in-memory disk-backed trie
    ///
    /// Reading from `current_binary_root` (in-memory, advanced per block by
    /// `apply_trie_updates`) is essential during catchup: the on-disk
    /// `META_ROOT_HASH` only updates on Phase-2 flushes which fire at the
    /// 128-layer threshold, so post-activation overlay writes from the first
    /// 127 blocks are not visible via disk-only paths (Bug 4, hoodi 2026-05-05).
    fn cache_get_leaf(
        &self,
        tree_key: &[u8; 32],
    ) -> Result<Option<Option<[u8; 32]>>, BinaryTrieError> {
        let cache = self
            .store
            .binary_trie_cache
            .read()
            .map_err(|_| BinaryTrieError::StoreError("binary_trie_cache RwLock poisoned".into()))?
            .clone();
        let state_root = self.store.current_binary_root().0;
        Ok(cache.get(state_root, tree_key))
    }
}

impl StoreBinaryTrieProvider {
    /// Load the 32-byte state root hash stored by the last successful commit.
    ///
    /// Returns `None` if the DB is empty (no commits yet).
    /// The hash is written as `META_ROOT_HASH` by `NodeStore::take_dirty`.
    pub(crate) fn load_current_root_hash(&self) -> Result<Option<[u8; 32]>, StoreError> {
        let tx = self.store.backend.begin_read()?;
        let bytes = tx.get(BINARY_TRIE_NODES, META_ROOT_HASH)?;
        match bytes {
            None => Ok(None),
            Some(b) if b.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&b);
                Ok(Some(arr))
            }
            Some(b) => Err(StoreError::Custom(format!(
                "META_ROOT_HASH has unexpected length {} (expected 32)",
                b.len()
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// impl Store: binary trie factory methods
// ---------------------------------------------------------------------------

impl Store {
    /// Create a [`BinaryTrieProvider`] backed by this store.
    ///
    /// Passed to `BinaryMerkleizer::new` and `BinaryBackend::new_with_db` as the
    /// dep-inversion seam for loading persisted trie nodes from `BINARY_TRIE_NODES`.
    pub fn make_binary_trie_provider(&self) -> Arc<dyn BinaryTrieProvider> {
        Arc::new(StoreBinaryTrieProvider {
            store: self.clone(),
        })
    }

    /// Create a read-only [`StateBackend`] for binary trie reads, anchored at `root`.
    ///
    /// The `root` is the `H256` state root of the block to read. This method:
    /// 1. Validates that `root` matches the on-disk tip (`META_ROOT_HASH`). Callers
    ///    must pass the real committed root; any zero or mismatched root is an error.
    /// 2. Opens a `BinaryTrieState` backed by `StorageTrieBackend` (a `TrieBackend`
    ///    adapter over this store's backend). This wires node loading from
    ///    `BINARY_TRIE_NODES` into the `NodeStore` so that reads traverse disk nodes.
    /// 3. Wraps the state in a `BinaryBackend` via `from_state`.
    ///
    /// The resulting reader sees the committed trie state. Reads for accounts or
    /// storage slots fall through to the DB if the node is not in memory.
    ///
    /// Historical root pinning (for roots other than the current tip) is not
    /// supported in Phase 5 — it requires a `H256 → NodeId` index that is out of
    /// scope until the archive/snapshot layer (Phase 7+).
    pub fn new_binary_state_reader(&self, root: H256) -> Result<StateBackend, StoreError> {
        let provider = StoreBinaryTrieProvider {
            store: self.clone(),
        };

        let stored_root_hash = provider.load_current_root_hash()?;

        // Validate that the requested root matches the stored tip.
        // Callers must supply the real committed root — no zero-bypass is allowed.
        match stored_root_hash {
            None => {
                return Err(StoreError::Custom(format!(
                    "binary trie has no committed state; cannot open reader at root {root:?}"
                )));
            }
            Some(h) if h != root.0 => {
                return Err(StoreError::Custom(format!(
                    "binary trie reader: requested root {root:?} does not match \
                     on-disk tip {:?}; historical root pinning is not supported in Phase 5",
                    H256(h)
                )));
            }
            _ => {} // root matches the stored tip — proceed.
        }

        // Build a DB-backed TrieBackend so NodeStore can lazily load nodes from disk.
        let trie_backend = Arc::new(StorageTrieBackend {
            store: self.clone(),
        });

        // Open a BinaryTrieState that uses the DB-backed NodeStore; this correctly
        // sets trie.root from META_ROOT and loads storage_keys if any exist.
        let binary_state =
            BinaryTrieState::open(trie_backend, BINARY_TRIE_NODES, BINARY_STORAGE_KEYS)
                .map_err(|e| StoreError::Custom(e.to_string()))?;

        let code_reader: CodeReader = self.make_code_reader();
        let provider_arc: Arc<dyn BinaryTrieProvider> = Arc::new(provider);

        Ok(StateBackend::Binary(BinaryBackend::from_state(
            binary_state,
            provider_arc,
            code_reader,
        )))
    }

    /// Create an in-memory [`StateBackend`] for binary trie writes.
    ///
    /// Used for unit tests and any bulk initialization path (genesis for binary
    /// is unsupported; this writer is for test/migration tooling only).
    ///
    /// All writes are in-memory; call `commit()` to get `NodeUpdates::Binary`
    /// for flushing to disk via `write_node_updates_direct`.
    pub fn new_binary_state_writer(&self) -> Result<StateBackend, StoreError> {
        let provider = self.make_binary_trie_provider();
        let code_reader = self.make_code_reader();
        Ok(StateBackend::new_binary_with_db(provider, code_reader))
    }
}

// ---------------------------------------------------------------------------
// binary_commit_nodes_to_disk
// ---------------------------------------------------------------------------

/// Write binary trie node diffs, stem tombstones, FKV leaf entries, and
/// SELFDESTRUCT FKV cleanup to disk in a single atomic write transaction.
///
/// # Arguments
///
/// - `backend`: storage backend to write to.
/// - `node_diffs`: raw `(key, value)` pairs from `NodeStore::take_dirty`. A
///   pair with an empty value means "delete this key from `BINARY_TRIE_NODES`".
/// - `deleted_stems`: 31-byte stems that were SELFDESTRUCTed. For each stem:
///   - A tombstone entry `[0xFE, stem...]` is written to `BINARY_TRIE_NODES`
///     with an empty value (presence-only marker; the cache-layer sentinel is distinct).
///   - ALL existing `BINARY_FLATKEYVALUE` entries whose key shares that stem
///     prefix (`key[0..31] == stem`) are deleted, ensuring that a
///     SELFDESTRUCTed account leaves no stale leaf values behind.
/// - `fkv_entries`: leaf-level diffs for `BINARY_FLATKEYVALUE`. Key is the
///   32-byte tree key `stem || sub_index`. Value is `Some(leaf)` for insert /
///   update, `None` for deletion.
///
/// **SELFDESTRUCT FKV cleanup**: before opening the write batch, the function
/// performs a prefix scan of `BINARY_FLATKEYVALUE` for each deleted stem to
/// enumerate all currently-occupied sub-indices. The scan happens on a
/// point-in-time read snapshot (before the write), so no TOCTOU race occurs.
/// All discovered keys are deleted inside the same write batch as the rest of
/// the commit.
///
/// **Last-write-wins**: when `fkv_entries` contains the same key more than once
/// (possible in BAL mode), the last entry for each key wins. The final map is
/// committed atomically.
///
/// # Atomicity
///
/// All writes (nodes, tombstones, FKV updates, FKV cleanup) share one
/// `begin_write` / `commit` block. If any write fails, the transaction is not
/// committed and no data lands.
pub(crate) fn binary_commit_nodes_to_disk(
    backend: &dyn StorageBackend,
    node_diffs: Vec<(Vec<u8>, Vec<u8>)>,
    deleted_stems: Vec<[u8; 31]>,
    fkv_entries: Vec<([u8; 32], Option<[u8; 32]>)>,
) -> Result<(), StoreError> {
    // Pre-scan: for each SELFDESTRUCTed stem, enumerate all existing FKV keys
    // that share its 31-byte prefix. This must happen BEFORE the write batch
    // so we read the current DB state (not the pending writes).
    // We collect into a Vec to avoid holding the read snapshot across the write.
    let mut selfdestruct_fkv_keys: Vec<[u8; 32]> = Vec::new();
    if !deleted_stems.is_empty() {
        let read_snap = backend.begin_read()?;
        for stem in &deleted_stems {
            let iter = read_snap.prefix_iterator(BINARY_FLATKEYVALUE, stem)?;
            for result in iter {
                let (key, _val) = result?;
                if key.len() == 32 && key[..31] == *stem {
                    let mut k = [0u8; 32];
                    k.copy_from_slice(&key);
                    selfdestruct_fkv_keys.push(k);
                }
            }
        }
    }

    let mut write_tx = backend.begin_write()?;

    // 1. Write trie node diffs to BINARY_TRIE_NODES.
    //    Empty value = delete; non-empty = put.
    for (key, value) in node_diffs {
        if value.is_empty() {
            write_tx.delete(BINARY_TRIE_NODES, &key)?;
        } else {
            write_tx.put(BINARY_TRIE_NODES, &key, &value)?;
        }
    }

    // 2. Write stem tombstones to BINARY_TRIE_NODES.
    //    Key: [0xFE, stem[0..31]] = 32 bytes.
    //    Value: empty slice (presence-only marker at the persistence layer).
    for stem in &deleted_stems {
        let key = tombstone_key(stem);
        write_tx.put(BINARY_TRIE_NODES, &key, &[])?;
    }

    // 3. Delete all pre-existing BINARY_FLATKEYVALUE entries for SELFDESTRUCTed
    //    stems (enumerated in the pre-scan above).
    for key in selfdestruct_fkv_keys {
        write_tx.delete(BINARY_FLATKEYVALUE, &key)?;
    }

    // 4. Write FKV leaf entries to BINARY_FLATKEYVALUE.
    //    Apply last-write-wins: collect into a BTreeMap (sorted by key) then write.
    //    (Duplicates only arise in BAL mode with non-deduplicated input.)
    //    BTreeMap is used instead of FxHashMap so that iteration order is
    //    deterministic (lexicographic by the 32-byte tree key). This ensures
    //    that write order is reproducible for audit logs and replay.
    let mut fkv_map: std::collections::BTreeMap<[u8; 32], Option<[u8; 32]>> =
        std::collections::BTreeMap::new();
    for (key, value) in fkv_entries {
        fkv_map.insert(key, value);
    }
    for (key, value) in fkv_map {
        match value {
            Some(leaf) => write_tx.put(BINARY_FLATKEYVALUE, &key, &leaf)?,
            None => write_tx.delete(BINARY_FLATKEYVALUE, &key)?,
        }
    }

    write_tx.commit()
}

// ---------------------------------------------------------------------------
// build_binary_cache_layer
// ---------------------------------------------------------------------------

/// Build a framed byte-key-value layer for the trie layer cache from binary
/// trie node diffs and stem tombstones.
///
/// # Framing
///
/// Values are framed using two tags (defined in `ethrex_binary_trie::hash`):
/// - `[CACHE_VALUE_TAG (0x00), ...node_bytes]` — a real node value.
/// - `[CACHE_TOMBSTONE_TAG (0x01)]` — a single-byte tombstone sentinel.
///
/// Empty `Vec<u8>` is **never** used as a sentinel; a missing key (`None` from
/// the layer cache) means "not in any layer, fall through to disk".
///
/// # Key format
///
/// - Node diff keys: raw keys from `NodeStore::take_dirty` (8-byte NodeId LE
///   or `0xFF`-prefixed metadata keys).
/// - Tombstone keys: `[0xFE, stem[0..31]]` = 32 bytes, matching the on-disk
///   format in `BINARY_TRIE_NODES`.
pub(crate) fn build_binary_cache_layer(
    node_diffs: Vec<(Vec<u8>, Vec<u8>)>,
    deleted_stems: Vec<[u8; 31]>,
) -> Vec<(Vec<u8>, Vec<u8>)> {
    let node_count = node_diffs.len();
    let tombstone_count = deleted_stems.len();
    let mut result = Vec::with_capacity(node_count + tombstone_count);

    // Frame node diffs.
    for (key, value) in node_diffs {
        let framed_value = if value.is_empty() {
            // Deletion: framed as single tombstone byte.
            vec![CACHE_TOMBSTONE_TAG]
        } else {
            // Real value: prefix with CACHE_VALUE_TAG.
            let mut framed = Vec::with_capacity(1 + value.len());
            framed.push(CACHE_VALUE_TAG);
            framed.extend_from_slice(&value);
            framed
        };
        result.push((key, framed_value));
    }

    // Frame stem tombstones.
    for stem in deleted_stems {
        let key = tombstone_key(&stem).to_vec();
        result.push((key, vec![CACHE_TOMBSTONE_TAG]));
    }

    result
}

/// Decode a framed value from the binary layer cache.
///
/// Returns:
/// - `Ok(Some(bytes))` — real value with the `0x00` prefix stripped.
/// - `Ok(None)` — tombstone (`[0x01]` single byte).
/// - `Err(StoreError)` — malformed framing (empty or unknown tag byte).
pub(crate) fn decode_binary_cache_value(framed: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
    match framed.first() {
        Some(&tag) if tag == CACHE_VALUE_TAG => Ok(Some(framed[1..].to_vec())),
        Some(&tag) if tag == CACHE_TOMBSTONE_TAG => {
            if framed.len() != 1 {
                return Err(StoreError::Custom(format!(
                    "binary cache tombstone has unexpected length {} (expected 1)",
                    framed.len()
                )));
            }
            Ok(None)
        }
        Some(&tag) => Err(StoreError::Custom(format!(
            "binary cache value has unknown tag byte 0x{tag:02x}"
        ))),
        None => Err(StoreError::Custom(
            "binary cache value is empty (empty Vec<u8> is not a valid sentinel)".to_string(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_binary_trie::hash::{CACHE_TOMBSTONE_TAG, CACHE_VALUE_TAG};

    // -----------------------------------------------------------------------
    // Task 5.1 smoke test: BINARY_TRIE_NODES and BINARY_FLATKEYVALUE exist
    // -----------------------------------------------------------------------

    /// Verify that both new tables are present in TABLES and can be written to
    /// via an InMemoryBackend (which creates tables on demand).
    #[test]
    fn test_tables_include_binary_tables() {
        use crate::api::tables::TABLES;
        assert!(
            TABLES.contains(&BINARY_TRIE_NODES),
            "TABLES must include BINARY_TRIE_NODES"
        );
        assert!(
            TABLES.contains(&BINARY_FLATKEYVALUE),
            "TABLES must include BINARY_FLATKEYVALUE"
        );
        assert!(
            TABLES.contains(&BINARY_STORAGE_KEYS),
            "TABLES must include BINARY_STORAGE_KEYS"
        );
        assert_eq!(TABLES.len(), 22, "TABLES must have exactly 22 entries");
    }

    #[test]
    fn test_inmemory_backend_can_write_binary_tables() {
        use crate::backend::in_memory::InMemoryBackend;
        let backend = InMemoryBackend::open().unwrap();
        let mut tx = backend.begin_write().unwrap();
        tx.put(BINARY_TRIE_NODES, &[0xFFu8, b'R'], &[1u8, 2, 3])
            .unwrap();
        tx.put(BINARY_FLATKEYVALUE, &[0u8; 32], &[42u8; 32])
            .unwrap();
        tx.commit().unwrap();

        let read = backend.begin_read().unwrap();
        let node_val = read
            .get(BINARY_TRIE_NODES, &[0xFFu8, b'R'])
            .unwrap()
            .unwrap();
        assert_eq!(node_val, vec![1u8, 2, 3]);
        let fkv_val = read.get(BINARY_FLATKEYVALUE, &[0u8; 32]).unwrap().unwrap();
        assert_eq!(fkv_val, vec![42u8; 32]);
    }

    // -----------------------------------------------------------------------
    // Task 5.6 / 5.12: framing round-trip tests
    // -----------------------------------------------------------------------

    /// Confirm that `[0x00, ..bytes]` decodes as a real value with the prefix stripped.
    #[test]
    fn test_decode_cache_value() {
        let node_bytes = vec![0xABu8, 0xCD, 0xEF];
        let mut framed = vec![CACHE_VALUE_TAG];
        framed.extend_from_slice(&node_bytes);
        let decoded = decode_binary_cache_value(&framed).unwrap();
        assert_eq!(decoded, Some(node_bytes));
    }

    /// Confirm that `[0x01]` decodes as a tombstone.
    ///
    /// Note (Deviation 1): `BinaryTrieLayerCache` is used as a FKV leaf cache
    /// (raw `Option<[u8; 32]>` values, no framing). The framing tested here
    /// (`CACHE_VALUE_TAG` / `CACHE_TOMBSTONE_TAG`) is used by `TrieLayerCache`
    /// for the binary node layer, not by `BinaryTrieLayerCache::get`. Tests of
    /// `BinaryTrieLayerCache` are in `apply_trie_updates` integration tests.
    #[test]
    fn test_decode_cache_tombstone() {
        let framed = vec![CACHE_TOMBSTONE_TAG];
        let decoded = decode_binary_cache_value(&framed).unwrap();
        assert!(decoded.is_none(), "tombstone must decode to None");
    }

    /// Confirm that an empty Vec is rejected.
    #[test]
    fn test_decode_empty_is_error() {
        let result = decode_binary_cache_value(&[]);
        assert!(
            result.is_err(),
            "empty value must be a decode error, not a sentinel"
        );
    }

    /// Confirm that an unknown tag byte is rejected.
    #[test]
    fn test_decode_unknown_tag_is_error() {
        let result = decode_binary_cache_value(&[0x02u8, 0x00]);
        assert!(result.is_err(), "unknown tag must be a decode error");
    }

    /// Round-trip: `build_binary_cache_layer` produces correctly framed entries.
    #[test]
    fn test_build_binary_cache_layer_framing() {
        let node_diffs = vec![
            (
                vec![0x01u8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
                vec![0x02, 0xAB, 0xCD],
            ),
            // Empty value = deletion.
            (
                vec![0x02u8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
                vec![],
            ),
        ];
        let stem = [0x55u8; 31];
        let deleted_stems = vec![stem];

        let layer = build_binary_cache_layer(node_diffs.clone(), deleted_stems);

        // First node: real value, framed with 0x00 prefix.
        let (k0, v0) = &layer[0];
        assert_eq!(k0, &node_diffs[0].0);
        assert_eq!(v0[0], CACHE_VALUE_TAG);
        assert_eq!(&v0[1..], &[0x02u8, 0xAB, 0xCD]);

        // Second node: deletion, framed as tombstone.
        let (k1, v1) = &layer[1];
        assert_eq!(k1, &node_diffs[1].0);
        assert_eq!(v1, &[CACHE_TOMBSTONE_TAG]);

        // Tombstone entry for the stem.
        let (k2, v2) = &layer[2];
        assert_eq!(k2[0], 0xFE);
        assert_eq!(&k2[1..], &stem);
        assert_eq!(v2, &[CACHE_TOMBSTONE_TAG]);

        // All entries are decodable.
        for (_, v) in &layer {
            assert!(decode_binary_cache_value(v).is_ok());
        }
    }

    // -----------------------------------------------------------------------
    // Task 5.5: binary_commit_nodes_to_disk atomicity and correctness
    // -----------------------------------------------------------------------

    /// Verify that nodes, tombstones, and FKV entries are all written atomically.
    #[test]
    fn test_binary_commit_nodes_to_disk_basic() {
        use crate::backend::in_memory::InMemoryBackend;
        let backend = InMemoryBackend::open().unwrap();

        // Write one node, one tombstone, two FKV entries (one insert, one delete setup).
        let node_diffs = vec![(vec![0x01u8, 0, 0, 0, 0, 0, 0, 0], vec![0x02, 0xAA])];
        let stem = [0x42u8; 31];
        let deleted_stems = vec![stem];
        let fkv_key1 = [0x11u8; 32];
        let fkv_val1 = [0xBBu8; 32];
        let fkv_entries = vec![(fkv_key1, Some(fkv_val1))];

        binary_commit_nodes_to_disk(&backend, node_diffs, deleted_stems, fkv_entries).unwrap();

        let read = backend.begin_read().unwrap();

        // Node is written.
        let node_val = read
            .get(BINARY_TRIE_NODES, &[0x01u8, 0, 0, 0, 0, 0, 0, 0])
            .unwrap()
            .unwrap();
        assert_eq!(node_val, vec![0x02, 0xAA]);

        // Tombstone is written.
        let tomb_key = tombstone_key(&stem);
        let tomb_val = read.get(BINARY_TRIE_NODES, &tomb_key).unwrap().unwrap();
        assert_eq!(tomb_val, vec![] as Vec<u8>);

        // FKV entry is written.
        let fkv_val = read.get(BINARY_FLATKEYVALUE, &fkv_key1).unwrap().unwrap();
        assert_eq!(fkv_val, fkv_val1.to_vec());
    }

    /// Verify last-write-wins on duplicate FKV keys.
    ///
    /// The same key appears twice in `fkv_entries`; the second (last) value
    /// must be the one stored. The intermediate value must NOT appear.
    #[test]
    fn test_binary_commit_fkv_last_write_wins() {
        use crate::backend::in_memory::InMemoryBackend;
        let backend = InMemoryBackend::open().unwrap();

        let key = [0x01u8; 32];
        let val_first = [0xAAu8; 32];
        let val_last = [0xBBu8; 32];
        // Same key appears twice; last write must win.
        let fkv_entries = vec![(key, Some(val_first)), (key, Some(val_last))];

        binary_commit_nodes_to_disk(&backend, vec![], vec![], fkv_entries).unwrap();

        let read = backend.begin_read().unwrap();
        let stored = read.get(BINARY_FLATKEYVALUE, &key).unwrap().unwrap();
        // Last-write-wins: val_last ([0xBB; 32]) must win; val_first ([0xAA; 32]) must NOT appear.
        assert_eq!(
            stored,
            val_last.to_vec(),
            "last-write-wins: the second value (0xBB) must be stored, not the first (0xAA)"
        );
        assert_ne!(
            stored,
            val_first.to_vec(),
            "first value (0xAA) must be overwritten by last-write-wins"
        );
    }

    /// Verify FKV deletion removes the key.
    #[test]
    fn test_binary_commit_fkv_deletion() {
        use crate::backend::in_memory::InMemoryBackend;
        let backend = InMemoryBackend::open().unwrap();

        let key = [0x01u8; 32];
        let val = [0xAAu8; 32];

        // First write: insert.
        binary_commit_nodes_to_disk(&backend, vec![], vec![], vec![(key, Some(val))]).unwrap();

        // Second write: delete.
        binary_commit_nodes_to_disk(&backend, vec![], vec![], vec![(key, None)]).unwrap();

        let read = backend.begin_read().unwrap();
        let stored = read.get(BINARY_FLATKEYVALUE, &key).unwrap();
        assert!(stored.is_none(), "deleted FKV key must not be present");
    }

    // -----------------------------------------------------------------------
    // Task 5.9: round-trip (Store::new_binary_state_writer → commit → reopen)
    // -----------------------------------------------------------------------

    /// Round-trip: write binary state, commit node diffs directly, reopen via
    /// `new_binary_state_reader(root)` and verify account reads return the
    /// committed state.
    #[test]
    fn test_binary_state_writer_round_trip() {
        use crate::{EngineType, Store};
        use ethrex_common::{Address, H256, U256, types::AccountInfo};
        use ethrex_state_backend::{
            AccountMut, BackendKind, NodeUpdates, StateCommitter, StateReader,
        };

        let store = Store::new(
            std::path::Path::new(""),
            EngineType::InMemory,
            BackendKind::Mpt,
        )
        .unwrap();

        let addr = Address::from([0x01u8; 20]);
        let mut backend = store.new_binary_state_writer().unwrap();

        // Insert an account.
        let info = AccountInfo {
            balance: U256::from(12345u64),
            nonce: 7,
            code_hash: *ethrex_common::constants::EMPTY_KECCACK_HASH,
        };
        backend
            .update_accounts(
                &[addr],
                &[AccountMut {
                    account: Some(info),
                    code: None,
                }],
            )
            .unwrap();
        let output = backend.commit().unwrap();
        let committed_root = output.root;

        // Commit node diffs directly to disk.
        let (node_diffs, deleted_stems, fkv_entries) = match output.node_updates {
            NodeUpdates::Binary {
                node_diffs,
                deleted_stems,
                fkv_entries,
            } => (node_diffs, deleted_stems, fkv_entries),
            NodeUpdates::Mpt { .. } => panic!("expected Binary variant"),
        };
        binary_commit_nodes_to_disk(
            store.backend.as_ref(),
            node_diffs,
            deleted_stems,
            fkv_entries.clone(),
        )
        .unwrap();

        // Verify FKV entries were written.
        assert!(
            !fkv_entries.is_empty(),
            "fkv_entries must be non-empty after account insert"
        );
        let read_raw = store.backend.begin_read().unwrap();
        for (key, value) in &fkv_entries {
            match value {
                Some(v) => {
                    let stored = read_raw.get(BINARY_FLATKEYVALUE, key).unwrap();
                    assert_eq!(stored, Some(v.to_vec()), "FKV entry must match");
                }
                None => {
                    let stored = read_raw.get(BINARY_FLATKEYVALUE, key).unwrap();
                    assert!(stored.is_none(), "deleted FKV key must be absent");
                }
            }
        }

        // Reopen via new_binary_state_reader(root) and verify account reads.
        let reader = store.new_binary_state_reader(committed_root).unwrap();
        let read_info = reader
            .account(addr)
            .unwrap()
            .expect("account must be readable via new_binary_state_reader after commit");
        assert_eq!(read_info.balance, info.balance, "balance must round-trip");
        assert_eq!(read_info.nonce, info.nonce, "nonce must round-trip");
        assert_eq!(
            read_info.code_hash, info.code_hash,
            "code_hash must round-trip"
        );

        // Passing a wrong (non-matching) root must be rejected.
        let wrong_root = H256::from([0xFFu8; 32]);
        assert!(
            store.new_binary_state_reader(wrong_root).is_err(),
            "new_binary_state_reader must reject a root that does not match the on-disk tip"
        );
    }

    // -----------------------------------------------------------------------
    // Task 5.10: inline FKV write (N stems → exact FKV entries)
    // -----------------------------------------------------------------------

    /// Apply N account updates, assert BINARY_FLATKEYVALUE has exactly the expected entries.
    #[test]
    fn test_inline_fkv_write() {
        use crate::{EngineType, Store};
        use ethrex_common::{Address, U256, types::AccountInfo};
        use ethrex_state_backend::{AccountMut, BackendKind, NodeUpdates, StateCommitter};

        let store = Store::new(
            std::path::Path::new(""),
            EngineType::InMemory,
            BackendKind::Mpt,
        )
        .unwrap();

        let n = 5u8;
        let mut backend = store.new_binary_state_writer().unwrap();

        for i in 0..n {
            let addr = Address::from([i + 1; 20]);
            let info = AccountInfo {
                balance: U256::from(u64::from(i) * 1000),
                nonce: u64::from(i),
                code_hash: *ethrex_common::constants::EMPTY_KECCACK_HASH,
            };
            backend
                .update_accounts(
                    &[addr],
                    &[AccountMut {
                        account: Some(info),
                        code: None,
                    }],
                )
                .unwrap();
        }

        let output = backend.commit().unwrap();
        let (node_diffs, deleted_stems, fkv_entries) = match output.node_updates {
            NodeUpdates::Binary {
                node_diffs,
                deleted_stems,
                fkv_entries,
            } => (node_diffs, deleted_stems, fkv_entries),
            NodeUpdates::Mpt { .. } => panic!("expected Binary"),
        };

        // Each account gets at least BASIC_DATA + CODE_HASH = 2 FKV entries.
        assert!(
            fkv_entries.len() >= usize::from(n) * 2,
            "expected at least {} FKV entries, got {}",
            usize::from(n) * 2,
            fkv_entries.len()
        );

        binary_commit_nodes_to_disk(
            store.backend.as_ref(),
            node_diffs,
            deleted_stems,
            fkv_entries.clone(),
        )
        .unwrap();

        // Verify all inserts are present (no deletions expected on fresh write).
        let read = store.backend.begin_read().unwrap();
        for (key, value) in &fkv_entries {
            if let Some(v) = value {
                let stored = read.get(BINARY_FLATKEYVALUE, key).unwrap();
                assert_eq!(stored, Some(v.to_vec()));
            }
        }
    }

    // -----------------------------------------------------------------------
    // Task 5.11: SELFDESTRUCT FKV cleanup
    // -----------------------------------------------------------------------

    /// Seed an account, commit (step 1), then SELFDESTRUCT it (step 2) using a
    /// writer that is properly anchored to the post-step-1 root. After step 2,
    /// assert that:
    ///   - the tombstone is present in BINARY_TRIE_NODES, AND
    ///   - ALL FKV entries for the stem are absent from BINARY_FLATKEYVALUE.
    #[test]
    fn test_selfdestruct_fkv_cleanup() {
        use crate::{EngineType, Store};
        use ethrex_binary_trie::key_mapping::get_stem_for_base;
        use ethrex_common::{Address, U256, types::AccountInfo};
        use ethrex_state_backend::{AccountMut, BackendKind, NodeUpdates, StateCommitter};

        let store = Store::new(
            std::path::Path::new(""),
            EngineType::InMemory,
            BackendKind::Mpt,
        )
        .unwrap();

        let addr = Address::from([0x07u8; 20]);
        let stem = get_stem_for_base(&addr);

        // Step 1: create the account and commit to disk.
        let step1_fkv_keys: Vec<[u8; 32]>;
        {
            let mut backend = store.new_binary_state_writer().unwrap();
            let info = AccountInfo {
                balance: U256::from(99999u64),
                nonce: 1,
                code_hash: *ethrex_common::constants::EMPTY_KECCACK_HASH,
            };
            backend
                .update_accounts(
                    &[addr],
                    &[AccountMut {
                        account: Some(info),
                        code: None,
                    }],
                )
                .unwrap();
            let output = backend.commit().unwrap();
            let (nd, ds, fkv) = match output.node_updates {
                NodeUpdates::Binary {
                    node_diffs,
                    deleted_stems,
                    fkv_entries,
                } => (node_diffs, deleted_stems, fkv_entries),
                _ => panic!("expected Binary"),
            };
            binary_commit_nodes_to_disk(store.backend.as_ref(), nd, ds, fkv.clone()).unwrap();

            // Collect the FKV keys that were written so we can assert they're gone later.
            step1_fkv_keys = fkv
                .iter()
                .filter_map(|(k, v)| if v.is_some() { Some(*k) } else { None })
                .collect();
            assert!(
                !step1_fkv_keys.is_empty(),
                "step 1 must produce at least one FKV entry (basic_data + code_hash)"
            );

            // Verify FKV written after step 1.
            let read = store.backend.begin_read().unwrap();
            for key in &step1_fkv_keys {
                assert!(
                    read.get(BINARY_FLATKEYVALUE, key).unwrap().is_some(),
                    "FKV must be present after create: {key:?}"
                );
            }
        }

        // Step 2: SELFDESTRUCT the account using a writer anchored to the tip
        // (which now contains the step-1 state via the DB-backed provider).
        {
            // new_binary_state_writer uses the DB provider, so it sees the
            // committed step-1 trie nodes. The SELFDESTRUCT removes the account
            // from the in-memory trie and adds the stem to deleted_stems.
            let mut backend = store.new_binary_state_writer().unwrap();
            backend
                .update_accounts(
                    &[addr],
                    &[AccountMut {
                        account: None,
                        code: None,
                    }],
                )
                .unwrap();
            let output = backend.commit().unwrap();
            let (nd, ds, fkv) = match output.node_updates {
                NodeUpdates::Binary {
                    node_diffs,
                    deleted_stems,
                    fkv_entries,
                } => (node_diffs, deleted_stems, fkv_entries),
                _ => panic!("expected Binary"),
            };

            // deleted_stems must contain our stem.
            assert!(
                ds.contains(&stem),
                "deleted_stems must include the SELFDESTRUCTed stem"
            );

            binary_commit_nodes_to_disk(store.backend.as_ref(), nd, ds.clone(), fkv.clone())
                .unwrap();

            let read = store.backend.begin_read().unwrap();

            // Tombstone must be present in BINARY_TRIE_NODES.
            for s in &ds {
                let tomb_key = tombstone_key(s);
                let found = read.get(BINARY_TRIE_NODES, &tomb_key).unwrap();
                assert!(
                    found.is_some(),
                    "tombstone must be present after SELFDESTRUCT"
                );
            }

            // ALL FKV entries for the stem must be absent (SELFDESTRUCT FKV cleanup).
            for key in &step1_fkv_keys {
                assert!(
                    read.get(BINARY_FLATKEYVALUE, key).unwrap().is_none(),
                    "FKV entry {key:?} must be deleted by SELFDESTRUCT FKV cleanup"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Task 5.13: atomic commit — no partial writes on failure
    //
    // Atomicity invariant: `binary_commit_nodes_to_disk` is a single
    // begin_write / commit block.  InMemoryBackend buffers all puts/deletes
    // in a Vec<PendingOp> and applies them to the shared store in a single
    // write-lock acquisition inside `commit()`.  Dropping the batch without
    // calling `commit()` discards the buffer — nothing lands.  This property
    // is tested in two scenarios:
    //
    //   (a) begin_write fails: the commit function returns early before any
    //       ops are queued.  Nothing lands in the backend.
    //
    //   (b) Nth put_batch fails (mid-batch): the write batch buffers the N-1
    //       successful puts but the Nth call returns an error.
    //       `binary_commit_nodes_to_disk` propagates the error without calling
    //       commit(), so the buffered ops are discarded when the batch is
    //       dropped.  No partial data lands in the underlying store.
    //
    // If someone re-introduces write-through behaviour in InMemoryWriteTx
    // (applying each put immediately rather than on commit), test (b) would
    // catch the regression because the pre-Nth ops would become visible before
    // the Nth failure, and the post-failure assertions would fire.
    // -----------------------------------------------------------------------

    /// A write batch that succeeds on the first N-1 `put_batch` calls and then
    /// returns an error.  Used to inject a mid-batch failure so we can verify
    /// that partially-queued ops are NOT applied to the underlying store.
    struct NthPutFailBatch {
        inner: Box<dyn crate::api::StorageWriteBatch + 'static>,
        fail_after: usize,
        call_count: usize,
    }

    impl crate::api::StorageWriteBatch for NthPutFailBatch {
        fn put_batch(
            &mut self,
            table: &'static str,
            batch: Vec<(Vec<u8>, Vec<u8>)>,
        ) -> Result<(), crate::error::StoreError> {
            self.call_count += 1;
            if self.call_count > self.fail_after {
                return Err(crate::error::StoreError::Custom(
                    "injected mid-batch put failure".to_string(),
                ));
            }
            self.inner.put_batch(table, batch)
        }

        fn delete(
            &mut self,
            table: &'static str,
            key: &[u8],
        ) -> Result<(), crate::error::StoreError> {
            self.inner.delete(table, key)
        }

        fn commit(&mut self) -> Result<(), crate::error::StoreError> {
            self.inner.commit()
        }
    }

    /// A `StorageBackend` that opens a real `InMemoryBackend` write batch but
    /// wraps it in `NthPutFailBatch` so that the Nth `put_batch` call fails.
    struct NthPutFailBackend {
        inner: crate::backend::in_memory::InMemoryBackend,
        fail_after: usize,
    }

    impl std::fmt::Debug for NthPutFailBackend {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "NthPutFailBackend(fail_after={})", self.fail_after)
        }
    }

    impl crate::api::StorageBackend for NthPutFailBackend {
        fn clear_table(&self, table: &'static str) -> Result<(), crate::error::StoreError> {
            self.inner.clear_table(table)
        }

        fn begin_read(
            &self,
        ) -> Result<std::sync::Arc<dyn crate::api::StorageReadView>, crate::error::StoreError>
        {
            self.inner.begin_read()
        }

        fn begin_write(
            &self,
        ) -> Result<Box<dyn crate::api::StorageWriteBatch + 'static>, crate::error::StoreError>
        {
            let inner_batch = self.inner.begin_write()?;
            Ok(Box::new(NthPutFailBatch {
                inner: inner_batch,
                fail_after: self.fail_after,
                call_count: 0,
            }))
        }

        fn begin_locked(
            &self,
            table_name: &'static str,
        ) -> Result<Box<dyn crate::api::StorageLockedView>, crate::error::StoreError> {
            self.inner.begin_locked(table_name)
        }

        fn create_checkpoint(
            &self,
            path: &std::path::Path,
        ) -> Result<(), crate::error::StoreError> {
            self.inner.create_checkpoint(path)
        }
    }

    /// `FailingBackend` wraps any `StorageBackend` and returns an error when
    /// `begin_write` is called. Tests scenario (a): begin_write fails before
    /// any ops are queued, so nothing lands.
    struct FailingBackend<B: crate::api::StorageBackend>(B);

    impl<B: crate::api::StorageBackend + std::fmt::Debug> std::fmt::Debug for FailingBackend<B> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "FailingBackend({:?})", self.0)
        }
    }

    impl<B: crate::api::StorageBackend + std::fmt::Debug + 'static> crate::api::StorageBackend
        for FailingBackend<B>
    {
        fn clear_table(&self, table: &'static str) -> Result<(), crate::error::StoreError> {
            self.0.clear_table(table)
        }

        fn begin_read(
            &self,
        ) -> Result<std::sync::Arc<dyn crate::api::StorageReadView>, crate::error::StoreError>
        {
            self.0.begin_read()
        }

        fn begin_write(
            &self,
        ) -> Result<Box<dyn crate::api::StorageWriteBatch + 'static>, crate::error::StoreError>
        {
            // Fail immediately: no write batch is created, so no data lands.
            Err(crate::error::StoreError::Custom(
                "injected write failure".to_string(),
            ))
        }

        fn begin_locked(
            &self,
            table_name: &'static str,
        ) -> Result<Box<dyn crate::api::StorageLockedView>, crate::error::StoreError> {
            self.0.begin_locked(table_name)
        }

        fn create_checkpoint(
            &self,
            path: &std::path::Path,
        ) -> Result<(), crate::error::StoreError> {
            self.0.create_checkpoint(path)
        }
    }

    /// Scenario (a): `begin_write` fails — assert no trie nodes or FKV entries
    /// land in the underlying database.
    ///
    /// This verifies the "all-or-nothing" atomicity claim: the entire commit
    /// is a single `begin_write/commit` block; if the batch cannot even be
    /// opened, nothing is written.
    #[test]
    fn test_binary_commit_no_partial_write_on_failure() {
        use crate::backend::in_memory::InMemoryBackend;
        let inner = InMemoryBackend::open().unwrap();

        // Pre-seed one entry so we have something to verify is NOT touched.
        binary_commit_nodes_to_disk(
            &inner,
            vec![(vec![0x01u8, 0, 0, 0, 0, 0, 0, 0], vec![0xAA])],
            vec![],
            vec![([0x11u8; 32], Some([0xBBu8; 32]))],
        )
        .unwrap();

        // Wrap the backend so all subsequent write batches fail.
        let failing = FailingBackend(InMemoryBackend::open().unwrap());

        // The new commit attempt targets a fresh (empty) FailingBackend.
        // It must return an error and leave the backend untouched.
        let node_key = vec![0x02u8, 0, 0, 0, 0, 0, 0, 0];
        let fkv_key = [0x22u8; 32];
        let result = binary_commit_nodes_to_disk(
            &failing,
            vec![(node_key.clone(), vec![0xBB])],
            vec![[0x42u8; 31]],
            vec![(fkv_key, Some([0xCCu8; 32]))],
        );
        assert!(result.is_err(), "commit must fail when begin_write fails");

        // The failing backend is empty — neither node nor FKV entry landed.
        let read = failing.0.begin_read().unwrap();
        assert!(
            read.get(BINARY_TRIE_NODES, &node_key).unwrap().is_none(),
            "no trie nodes must land when begin_write fails"
        );
        assert!(
            read.get(BINARY_FLATKEYVALUE, &fkv_key).unwrap().is_none(),
            "no FKV entries must land when begin_write fails"
        );

        // The original backend is also unaffected.
        let read2 = inner.begin_read().unwrap();
        assert!(
            read2
                .get(BINARY_TRIE_NODES, &[0x01u8, 0, 0, 0, 0, 0, 0, 0])
                .unwrap()
                .is_some(),
            "pre-seeded trie node must survive"
        );
        assert!(
            read2
                .get(BINARY_FLATKEYVALUE, &[0x11u8; 32])
                .unwrap()
                .is_some(),
            "pre-seeded FKV entry must survive"
        );
    }

    /// Scenario (b): the second `put_batch` call fails mid-batch.
    ///
    /// `binary_commit_nodes_to_disk` queues the first node diff successfully
    /// but then the second call (for the tombstone or FKV entry) returns an error.
    /// Because `InMemoryBackend` is buffered (all ops land atomically on `commit()`),
    /// dropping the batch without calling `commit()` discards the entire buffer —
    /// no partial data reaches the underlying store.
    ///
    /// This test catches a regression if someone re-introduces write-through
    /// behaviour (applying each put immediately rather than on commit).
    #[test]
    fn test_binary_commit_no_partial_write_mid_batch() {
        use crate::backend::in_memory::InMemoryBackend;

        // fail_after = 1: first put_batch succeeds, second fails.
        let fail_backend = NthPutFailBackend {
            inner: InMemoryBackend::open().unwrap(),
            fail_after: 1,
        };

        // Pass two node diffs so the second put_batch call will fail.
        let key_first = vec![0x01u8, 0, 0, 0, 0, 0, 0, 0];
        let key_second = vec![0x02u8, 0, 0, 0, 0, 0, 0, 0];
        let result = binary_commit_nodes_to_disk(
            &fail_backend,
            vec![
                (key_first.clone(), vec![0xAA]),
                (key_second.clone(), vec![0xBB]),
            ],
            vec![],
            vec![],
        );
        assert!(
            result.is_err(),
            "commit must fail when a mid-batch put fails"
        );

        // Neither the first (successful) nor the second (failed) op must land.
        let read = fail_backend.inner.begin_read().unwrap();
        assert!(
            read.get(BINARY_TRIE_NODES, &key_first).unwrap().is_none(),
            "first node must NOT land after mid-batch failure (buffered atomicity)"
        );
        assert!(
            read.get(BINARY_TRIE_NODES, &key_second).unwrap().is_none(),
            "second node must NOT land after mid-batch failure"
        );
    }

    /// Verify that a mid-batch `put` failure (via a write batch that fails on
    /// `commit`) leaves the database unchanged. Uses `InMemoryBackend`'s new
    /// buffered semantics: all puts are staged in memory and applied atomically
    /// on `commit()` — so if we never call `commit()`, nothing lands.
    ///
    /// We simulate this by creating a write batch, calling `put`, and then
    /// dropping the batch without committing. The backend must be empty.
    #[test]
    fn test_inmemory_batch_uncommitted_leaves_no_data() {
        use crate::backend::in_memory::InMemoryBackend;
        let backend = InMemoryBackend::open().unwrap();

        {
            let mut tx = backend.begin_write().unwrap();
            tx.put(BINARY_TRIE_NODES, &[0x01u8, 0, 0, 0, 0, 0, 0, 0], &[0xAA])
                .unwrap();
            tx.put(BINARY_FLATKEYVALUE, &[0x11u8; 32], &[0xBBu8; 32])
                .unwrap();
            // Drop `tx` without calling commit() — nothing must land.
        }

        let read = backend.begin_read().unwrap();
        assert!(
            read.get(BINARY_TRIE_NODES, &[0x01u8, 0, 0, 0, 0, 0, 0, 0])
                .unwrap()
                .is_none(),
            "uncommitted puts must not be visible"
        );
        assert!(
            read.get(BINARY_FLATKEYVALUE, &[0x11u8; 32])
                .unwrap()
                .is_none(),
            "uncommitted puts must not be visible"
        );
    }

    // -----------------------------------------------------------------------
    // Cache-walk regression tests for the binary read path.
    //
    // These cover the post-Bug-5 asymmetries: until they were fixed, the
    // overlay reader's stem-tombstone check and the in-memory trie traversal
    // both consulted disk only and missed in-memory cache layers, so a
    // SELFDESTRUCT or any node write that hadn't yet been Phase-2-flushed to
    // disk was invisible until the 128-layer threshold fired.
    // -----------------------------------------------------------------------

    /// Fix #1: `is_deleted_stem` walks the in-memory `trie_cache` at the live
    /// `current_binary_root` before falling through to disk.
    ///
    /// Setup: insert a synthetic binary cache layer containing only a stem
    /// tombstone, advance `current_binary_root` to that layer's root, leave
    /// disk empty. Pre-fix code would have read disk-only and returned false.
    #[test]
    fn is_deleted_stem_walks_cache_before_disk() {
        use crate::{EngineType, Store};
        use ethrex_state_backend::BackendKind;

        let store = Store::new(
            std::path::Path::new(""),
            EngineType::InMemory,
            BackendKind::Mpt,
        )
        .unwrap();

        let stem = [0xAAu8; 31];
        let head_root = H256::from([0x42u8; 32]);
        let parent_root = H256::zero();

        // Build a cache layer holding just this stem tombstone (no node
        // diffs). build_binary_cache_layer frames it as
        // ([0xFE, stem...; 32], [CACHE_TOMBSTONE_TAG; 1]).
        let layer = build_binary_cache_layer(Vec::new(), vec![stem]);
        {
            let cache = store.trie_cache.read().unwrap().clone();
            let mut cache_mut = (*cache).clone();
            cache_mut.put_batch(parent_root, head_root, layer);
            *store.trie_cache.write().unwrap() = Arc::new(cache_mut);
        }
        *store.current_binary_root.write().unwrap() = head_root;

        // Disk has no tombstone for this stem.
        let read = store.backend.begin_read().unwrap();
        let tomb_key = tombstone_key(&stem);
        assert!(
            read.get(BINARY_TRIE_NODES, &tomb_key).unwrap().is_none(),
            "precondition: tombstone must NOT be on disk yet (cache-only)"
        );

        // is_deleted_stem must report true via the cache walk.
        let provider = StoreBinaryTrieProvider {
            store: store.clone(),
        };
        assert!(
            provider.is_deleted_stem(&stem).unwrap(),
            "is_deleted_stem must walk trie_cache and find the tombstone before disk"
        );

        // A different stem (not in cache, not on disk) must still report false.
        let other_stem = [0xBBu8; 31];
        assert!(
            !provider.is_deleted_stem(&other_stem).unwrap(),
            "is_deleted_stem must return false for a stem with no cache or disk entry"
        );
    }

    /// Fix #3: `CacheAwareTrieBackend::get` for `BINARY_TRIE_NODES` consults
    /// the in-memory `trie_cache` at `current_binary_root` before falling
    /// through to disk; a node-id key written into a cache layer is
    /// retrievable even though it never reached disk.
    #[test]
    fn cache_aware_trie_backend_serves_node_from_cache() {
        use crate::{EngineType, Store};
        use ethrex_state_backend::BackendKind;

        let store = Store::new(
            std::path::Path::new(""),
            EngineType::InMemory,
            BackendKind::Mpt,
        )
        .unwrap();

        // Synthetic node-id key (8-byte LE) and node bytes.
        let node_id_key = vec![0x07u8, 0, 0, 0, 0, 0, 0, 0];
        let node_bytes = vec![0xDE, 0xAD, 0xBE, 0xEF];

        // Build cache layer with the node diff.
        let head_root = H256::from([0x55u8; 32]);
        let parent_root = H256::zero();
        let layer =
            build_binary_cache_layer(vec![(node_id_key.clone(), node_bytes.clone())], Vec::new());
        {
            let cache = store.trie_cache.read().unwrap().clone();
            let mut cache_mut = (*cache).clone();
            cache_mut.put_batch(parent_root, head_root, layer);
            *store.trie_cache.write().unwrap() = Arc::new(cache_mut);
        }
        *store.current_binary_root.write().unwrap() = head_root;

        // Precondition: disk does not have this node.
        let read = store.backend.begin_read().unwrap();
        assert!(
            read.get(BINARY_TRIE_NODES, &node_id_key).unwrap().is_none(),
            "precondition: node must NOT be on disk yet (cache-only)"
        );

        // Read via CacheAwareTrieBackend — must return the unframed node
        // bytes from the cache.
        let cache_aware = CacheAwareTrieBackend {
            store: store.clone(),
            inner: StorageTrieBackend {
                store: store.clone(),
            },
        };
        let got = cache_aware.get(BINARY_TRIE_NODES, &node_id_key).unwrap();
        assert_eq!(
            got,
            Some(node_bytes),
            "cache-aware backend must return unframed node bytes from cache"
        );

        // Reads on a different table bypass the cache (table != BINARY_TRIE_NODES).
        let other = cache_aware.get(BINARY_FLATKEYVALUE, &[0u8; 32]).unwrap();
        assert!(
            other.is_none(),
            "non-BINARY_TRIE_NODES tables must bypass the cache walk"
        );
    }
}
