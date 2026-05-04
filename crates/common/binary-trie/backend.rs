//! `BinaryBackend` — an implementation of [`StateReader`] + [`StateCommitter`]
//! backed by the EIP-7864 binary trie.
//!
//! This is the in-process state interface for the binary trie backend. It is
//! used by the non-pipelined code paths (genesis, snap sync, tests) and as the
//! inner store of `BinaryMerkleizer` in the pipelined path.
//!
//! # Stem-group write invariant
//!
//! Every write to an account **must** emit both `BASIC_DATA_LEAF_KEY` (sub-index
//! 0) and `CODE_HASH_LEAF_KEY` (sub-index 1) atomically via the internal
//! `insert_stem_group_internal` helper (for `BinaryBackend`'s own writes) or the
//! public `insert_stem_group` (for `TransitionBackend`'s CoW pulls).
//! Reads check the invariant with a `debug_assert!`.
//! This mirrors the overlay-stem integrity invariant from the plan §3.

use std::sync::Arc;

use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCACK_HASH,
    types::{AccountInfo, AccountUpdate, Code},
};
use ethrex_crypto::NativeCrypto;
use ethrex_state_backend::{
    AccountMut, BackendKind, CodeMut, CodeReader, MerkleOutput, NodeUpdates, StateCommitter,
    StateError, StateReader,
};
use rustc_hash::FxHashSet;

use crate::{
    BinaryTrieError, BinaryTrieState,
    key_mapping::{
        BASIC_DATA_LEAF_KEY, CODE_HASH_LEAF_KEY, chunkify_code, get_stem_for_base,
        get_tree_key_for_basic_data, get_tree_key_for_code_chunk, get_tree_key_for_code_hash,
        get_tree_key_for_storage_slot, pack_basic_data, tree_key_from_stem, unpack_basic_data,
    },
};

// ---------------------------------------------------------------------------
// BinaryTrieProvider trait
// ---------------------------------------------------------------------------

/// Dep-inversion seam between `BinaryBackend` and the storage layer.
///
/// A `BinaryTrieProvider` loads persisted trie nodes and tombstone markers.
/// The storage layer (`ethrex-storage`) implements this trait via
/// `StoreBinaryTrieProvider` (added in Phase 5). In-memory / genesis paths
/// use [`EmptyBinaryTrieProvider`] which returns `None`/`false` for everything.
pub trait BinaryTrieProvider: Send + Sync {
    /// Load a serialized trie node by its 8-byte node ID.
    fn load_node(&self, id: u64) -> Result<Option<Vec<u8>>, BinaryTrieError>;
    /// Load a metadata entry by raw key (e.g. `META_ROOT`, `META_NEXT_ID`, or a
    /// `0xFF`-prefixed custom key).
    fn load_meta(&self, key: &[u8]) -> Result<Option<Vec<u8>>, BinaryTrieError>;
    /// Returns `true` if the given 31-byte stem has a tombstone entry in the
    /// persistence layer (i.e. the account was SELFDESTRUCTed in a previous
    /// block and the tombstone was committed to disk).
    fn is_deleted_stem(&self, stem: &[u8; 31]) -> Result<bool, BinaryTrieError>;
    /// Returns `true` if the given 32-byte tree key has any FKV entry in the
    /// persistence layer (including an explicit `[0; 32]` zero marker written
    /// by a post-switch SSTORE 0). Used by the overlay (transition) read path
    /// to distinguish "explicitly zeroed in overlay" from "absent, fall
    /// through to base".
    fn is_slot_in_fkv(&self, tree_key: &[u8; 32]) -> Result<bool, BinaryTrieError>;
}

/// A no-op provider used for in-memory / genesis paths where there is no
/// DB-backed storage yet. Every query returns `None` / `false`.
pub struct EmptyBinaryTrieProvider;

impl BinaryTrieProvider for EmptyBinaryTrieProvider {
    fn load_node(&self, _id: u64) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        Ok(None)
    }

    fn load_meta(&self, _key: &[u8]) -> Result<Option<Vec<u8>>, BinaryTrieError> {
        Ok(None)
    }

    fn is_deleted_stem(&self, _stem: &[u8; 31]) -> Result<bool, BinaryTrieError> {
        Ok(false)
    }

    fn is_slot_in_fkv(&self, _tree_key: &[u8; 32]) -> Result<bool, BinaryTrieError> {
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// BinaryBackend struct
// ---------------------------------------------------------------------------

/// Implements [`StateReader`] + [`StateCommitter`] on top of [`BinaryTrieState`].
///
/// All writes to an account's stem go through
/// [`BinaryBackend::insert_stem_group_internal`] (internal) or
/// [`BinaryBackend::insert_stem_group`] (public CoW path) which atomically
/// insert **both** `BASIC_DATA_LEAF_KEY` and `CODE_HASH_LEAF_KEY`
/// (the stem-group write invariant from the plan §3).
pub struct BinaryBackend {
    /// Underlying binary trie holding all state leaves.
    state: BinaryTrieState,
    /// Code reader — delegates to the legacy `AccountCodes` table. Pre-switch
    /// AND post-switch code reads go through here; post-switch deploys are
    /// dual-written to the table so this always works.
    code_reader: CodeReader,
    /// Dep-inversion provider for loading persisted trie nodes and tombstones.
    provider: Arc<dyn BinaryTrieProvider>,
    /// In-memory tombstone set for SELFDESTRUCTed accounts within the current
    /// block batch. Drained into `NodeUpdates::Binary.deleted_stems` on `commit`.
    deleted_stems: FxHashSet<[u8; 31]>,
    /// Accumulated code deployments: `(code_hash, bytecode)` pairs for dual-write
    /// to the legacy `AccountCodes` table so code reads always succeed.
    code_updates: Vec<(H256, Code)>,
}

impl BinaryBackend {
    /// Create an empty, in-memory `BinaryBackend` with no DB backing.
    ///
    /// Suitable for unit tests and genesis paths where no persisted trie exists.
    pub fn new() -> Self {
        Self {
            state: BinaryTrieState::new(),
            code_reader: Arc::new(|_| Ok(None)),
            provider: Arc::new(EmptyBinaryTrieProvider),
            deleted_stems: FxHashSet::default(),
            code_updates: Vec::new(),
        }
    }

    /// Create a `BinaryBackend` backed by a DB provider and an explicit code reader.
    pub fn new_with_db(provider: Arc<dyn BinaryTrieProvider>, code_reader: CodeReader) -> Self {
        Self {
            state: BinaryTrieState::new(),
            code_reader,
            provider,
            deleted_stems: FxHashSet::default(),
            code_updates: Vec::new(),
        }
    }

    /// Create a `BinaryBackend` from an existing [`BinaryTrieState`] (e.g. after
    /// bulk-loading genesis data or replaying a witness).
    pub fn from_state(
        state: BinaryTrieState,
        provider: Arc<dyn BinaryTrieProvider>,
        code_reader: CodeReader,
    ) -> Self {
        Self {
            state,
            code_reader,
            provider,
            deleted_stems: FxHashSet::default(),
            code_updates: Vec::new(),
        }
    }

    /// Reconstruct a `BinaryBackend` from a `BinaryTrieWitness` by replaying the
    /// witness's proven pre-state leaves into a fresh trie.
    ///
    /// Trust model & scope:
    /// - **This constructor does not verify proofs.** Proof verification is the
    ///   caller's responsibility (e.g., call `BinaryTrieProof::verify` on each
    ///   entry against `witness.pre_state_root` before invoking this helper, or
    ///   rely on the stateless-execution layer's verifier).
    /// - The reconstructed trie is **partial**: it contains only the leaves
    ///   referenced by the witness (touched accounts + their touched storage
    ///   slots). Its `state_root()` will NOT match `witness.pre_state_root`
    ///   unless the witness happens to cover every leaf of the source trie
    ///   (which witnesses in general do not — they only carry accessed state).
    /// - Reads on addresses / storage slots that are NOT in the witness may
    ///   return `None` or `H256::zero()`. Callers that need to distinguish
    ///   "genuinely absent" from "not in this witness" must track that outside
    ///   the backend.
    /// - Witness `codes` are not inserted into the trie — they belong in the
    ///   external `AccountCodes` table. The caller is responsible for
    ///   populating a `code_reader` that serves them.
    ///
    /// Half-stem detection: if a witness entry has one of `basic_data_proof` /
    /// `code_hash_proof` populated and the other absent, the stem-group
    /// invariant is violated. This indicates a malformed source trie (one
    /// written outside `insert_stem_group`) or an adversarial witness.
    /// Reconstruction returns `BinaryTrieError::InvalidWitness` rather than
    /// produce a half-stem state that would fire the read-path
    /// `debug_assert!` in debug builds or silently misbehave in release.
    pub fn from_witness(
        witness: &crate::witness::BinaryTrieWitness,
        code_reader: CodeReader,
    ) -> Result<Self, BinaryTrieError> {
        let mut state = BinaryTrieState::new();

        // Replay account proofs with stem-group invariant enforcement:
        // both leaves must be present, or both absent.
        for entry in &witness.account_proofs {
            let stem = get_stem_for_base(&entry.address);
            match (entry.basic_data_proof.value, entry.code_hash_proof.value) {
                (Some(basic_data), Some(code_hash)) => {
                    state.trie_insert_multi(
                        stem,
                        &[
                            (BASIC_DATA_LEAF_KEY, basic_data),
                            (CODE_HASH_LEAF_KEY, code_hash),
                        ],
                    )?;
                }
                (None, None) => {
                    // Account absent from the source trie. Leave the stem empty.
                }
                _ => {
                    // Half-stem witness: one leaf present, the other missing.
                    return Err(BinaryTrieError::InvalidWitness);
                }
            }
        }

        // Replay storage proofs — insert the raw leaf value at the computed key.
        for entry in &witness.storage_proofs {
            if let Some(value) = entry.proof.value {
                let storage_key_u256 = U256::from_big_endian(entry.slot.as_bytes());
                let tree_key = get_tree_key_for_storage_slot(&entry.address, storage_key_u256);
                state.trie_insert(tree_key, value)?;
            }
        }

        Ok(Self {
            state,
            code_reader,
            provider: Arc::new(EmptyBinaryTrieProvider),
            deleted_stems: FxHashSet::default(),
            code_updates: Vec::new(),
        })
    }

    /// Identify which backend kind this produces. Used by callers that need to
    /// tag `NodeUpdates` without having a full `Store` reference.
    pub fn backend_kind(&self) -> BackendKind {
        BackendKind::Binary
    }

    // -----------------------------------------------------------------------
    // Stem-group helpers (enforce the write invariant)
    // -----------------------------------------------------------------------

    /// Check whether the overlay has a live (non-tombstoned) `BASIC_DATA_LEAF_KEY`
    /// entry for this stem.
    ///
    /// Returns `true` if the stem exists in the overlay and is not in the
    /// in-block `deleted_stems` set. Used by `TransitionBackend` to decide
    /// whether a CoW pull from the MPT base is needed.
    pub fn stem_has_basic_data(&self, stem: &[u8; 31]) -> Result<bool, StateError> {
        if self.deleted_stems.contains(stem) {
            return Ok(false);
        }
        let basic_key = tree_key_from_stem(stem, BASIC_DATA_LEAF_KEY);
        Ok(self.state.trie_get(basic_key).is_some())
    }

    /// Check whether this stem is currently tombstoned in the overlay.
    ///
    /// Checks both the in-block `deleted_stems` set (in-memory tombstones from
    /// the current session) and the persistence layer via the provider
    /// (`is_deleted_stem`). After a process restart `deleted_stems` is empty;
    /// the provider lookup ensures persisted tombstones remain visible.
    pub fn stem_is_tombstoned(&self, stem: &[u8; 31]) -> Result<bool, StateError> {
        if self.deleted_stems.contains(stem) {
            return Ok(true);
        }
        self.provider
            .is_deleted_stem(stem)
            .map_err(|e| StateError::Other(e.to_string()))
    }

    /// Write a tombstone for `stem` into the in-memory `deleted_stems` set
    /// without removing any account or storage leaves from the trie.
    ///
    /// Used by `TransitionBackend::clear_storage` to mark a stem as
    /// storage-wiped (so MPT base storage is hidden) independently of whether
    /// the account itself is being removed. The tombstone is drained into
    /// `NodeUpdates::Binary.deleted_stems` on `commit` and persisted to disk.
    pub fn tombstone_stem(&mut self, stem: &[u8; 31]) {
        self.deleted_stems.insert(*stem);
    }

    /// Returns `true` if a storage slot has any record in the overlay — either
    /// a non-zero value in the trie OR an explicit zero marker in the FKV
    /// (current session via `current_block_diffs`, or persisted on disk).
    ///
    /// The overlay (transition) read path uses this to distinguish "explicitly
    /// zeroed in overlay" from "absent in overlay, fall through to base".
    /// Without this, a post-switch SSTORE 0 to a slot with a non-zero pre-switch
    /// value would resurrect the pre-switch value on subsequent reads.
    pub fn slot_is_in_overlay(&self, addr: Address, slot: H256) -> Result<bool, StateError> {
        let storage_key = U256::from_big_endian(slot.as_bytes());
        let tree_key = get_tree_key_for_storage_slot(&addr, storage_key);

        // Non-zero in trie → in overlay.
        if self.state.trie_get(tree_key).is_some() {
            return Ok(true);
        }
        // Recorded as a zero-write in this session's pending diffs.
        if self.state.has_pending_fkv_entry(&tree_key) {
            return Ok(true);
        }
        // Persisted in FKV from a prior block (zero or non-zero).
        self.provider
            .is_slot_in_fkv(&tree_key)
            .map_err(|e| StateError::Other(e.to_string()))
    }

    /// Return the `code_size` stored in the `BASIC_DATA` leaf for `addr`, or 0 if absent.
    ///
    /// Used by tests and `update_accounts` to read the current `code_size` without
    /// exposing the internal `BinaryTrieState` field.
    pub fn get_code_size(&self, addr: &Address) -> u32 {
        self.state.get_code_size(addr)
    }

    /// Atomically insert a set of `(sub_index, value)` leaves on a single stem.
    ///
    /// Used for CoW pulls from the MPT base into the binary overlay. The caller
    /// supplies the exact leaf pairs; this helper performs a single trie walk for
    /// all of them. Records FKV diffs for every inserted leaf.
    ///
    /// For account stems the caller must always include at least
    /// `BASIC_DATA_LEAF_KEY` and `CODE_HASH_LEAF_KEY` to uphold the stem-group
    /// write invariant.
    pub fn insert_stem_group(
        &mut self,
        stem: &[u8; 31],
        leaves: &[(u8, [u8; 32])],
    ) -> Result<(), StateError> {
        self.state
            .trie_insert_multi(*stem, leaves)
            .map_err(|e| StateError::Other(e.to_string()))?;
        // Record FKV diffs for each leaf so they propagate to BINARY_FLATKEYVALUE.
        for (sub_index, value) in leaves {
            self.state
                .record_insert(tree_key_from_stem(stem, *sub_index), *value);
        }
        Ok(())
    }

    /// Atomically insert both `BASIC_DATA_LEAF_KEY` and `CODE_HASH_LEAF_KEY` for
    /// `stem` in a single trie walk.
    ///
    /// This is the only permitted path for account creation / mutation — it
    /// enforces the stem-group write invariant: the two leaves are always
    /// present together or both absent.
    ///
    /// Internal helper; external callers use `insert_stem_group` which takes an
    /// arbitrary `&[(u8, [u8; 32])]` slice.
    fn insert_stem_group_internal(
        &mut self,
        stem: &[u8; 31],
        basic_data: [u8; 32],
        code_hash: [u8; 32],
    ) -> Result<(), BinaryTrieError> {
        self.state.trie_insert_multi(
            *stem,
            &[
                (BASIC_DATA_LEAF_KEY, basic_data),
                (CODE_HASH_LEAF_KEY, code_hash),
            ],
        )
    }

    // -----------------------------------------------------------------------
    // Code chunking
    // -----------------------------------------------------------------------

    /// Chunkify bytecode into EIP-7864 32-byte chunks.
    ///
    /// Returns `Vec<(tree_key, chunk_value)>` suitable for direct insertion
    /// into the binary trie. Does NOT insert them — callers must apply the
    /// returned pairs.
    pub fn code_chunks_from_bytecode(
        address: &Address,
        bytecode: &[u8],
    ) -> Vec<([u8; 32], [u8; 32])> {
        let chunks = chunkify_code(bytecode);
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, chunk)| {
                let key = get_tree_key_for_code_chunk(address, i as u64);
                (key, chunk)
            })
            .collect()
    }
}

impl Default for BinaryBackend {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// StateReader
// ---------------------------------------------------------------------------

impl StateReader for BinaryBackend {
    /// Read an account from the binary trie.
    ///
    /// Returns `None` if the account does not exist in the trie (both
    /// `BASIC_DATA_LEAF_KEY` and `CODE_HASH_LEAF_KEY` are absent).
    ///
    /// Stem-group invariant enforced by `debug_assert!`: if one leaf is present
    /// then the other must be too.
    fn account(&self, addr: Address) -> Result<Option<AccountInfo>, StateError> {
        let basic_data_key = get_tree_key_for_basic_data(&addr);
        let code_hash_key = get_tree_key_for_code_hash(&addr);

        let basic_data = self.state.trie_get(basic_data_key);
        let code_hash_raw = self.state.trie_get(code_hash_key);

        // Stem-group invariant: both present or both absent.
        debug_assert!(
            basic_data.is_some() == code_hash_raw.is_some(),
            "stem-group invariant violated for {addr:?}: \
             basic_data={}, code_hash={}",
            basic_data.is_some(),
            code_hash_raw.is_some(),
        );

        let Some(bd) = basic_data else {
            return Ok(None);
        };

        let (_version, _code_size, nonce, balance) = unpack_basic_data(&bd);
        let code_hash = code_hash_raw.map(H256).unwrap_or(*EMPTY_KECCACK_HASH);

        // `AccountInfo` does not carry `code_size`; callers that need it use
        // `BinaryTrieState::get_code_size`. The value is stored in `basic_data`
        // and reconstructed on demand. This matches MPT behaviour.
        Ok(Some(AccountInfo {
            nonce,
            balance,
            code_hash,
        }))
    }

    /// Read a storage slot value. Returns `H256::zero()` if absent.
    fn storage(&self, addr: Address, slot: H256) -> Result<H256, StateError> {
        let storage_key = U256::from_big_endian(slot.as_bytes());
        let tree_key = get_tree_key_for_storage_slot(&addr, storage_key);
        Ok(self.state.trie_get(tree_key).map(H256).unwrap_or_default())
    }

    /// Read code by hash. Delegates to the `code_reader` (legacy `AccountCodes`
    /// table). Post-switch deploys are dual-written there (see `update_accounts`),
    /// so this path always works. No chunk-reconstruction is performed.
    fn code(&self, _addr: Address, code_hash: H256) -> Result<Option<Vec<u8>>, StateError> {
        (self.code_reader)(code_hash)
    }
}

// ---------------------------------------------------------------------------
// StateCommitter
// ---------------------------------------------------------------------------

impl StateCommitter for BinaryBackend {
    /// Apply account mutations.
    ///
    /// For each `(addr, acct_mut)` pair:
    /// - `None` account (SELFDESTRUCT): add stem to `deleted_stems`; remove all
    ///   leaves under the base stem from the trie.
    /// - `Some(info)`: pack `basic_data`, insert both leaves atomically via
    ///   `insert_stem_group`.  When `acct_mut.code` is `Some`, also insert code
    ///   chunks and dual-write `(code_hash, bytecode)` to `code_updates`.
    fn update_accounts(
        &mut self,
        addrs: &[Address],
        muts: &[AccountMut],
    ) -> Result<(), StateError> {
        for (addr, acct_mut) in addrs.iter().zip(muts.iter()) {
            match &acct_mut.account {
                None => {
                    // SELFDESTRUCT: tombstone the stem and wipe all trie leaves.
                    let stem = get_stem_for_base(addr);
                    self.deleted_stems.insert(stem);

                    // Build an AccountUpdate so we can reuse BinaryTrieState::apply_account_update.
                    let mut upd = AccountUpdate::new(*addr);
                    upd.removed = true;
                    self.state
                        .apply_account_update(&upd)
                        .map_err(|e| StateError::Other(e.to_string()))?;
                }
                Some(info) => {
                    let stem = get_stem_for_base(addr);

                    // If this account was previously selfdestructed in this block,
                    // clear the tombstone — writing Some(info) means the account
                    // now exists again (e.g. post-selfdestruct re-creation).
                    self.deleted_stems.remove(&stem);

                    // Determine code_size: use new bytecode length when deploying;
                    // preserve existing trie value for all other updates (including
                    // `Some(CodeMut { code: None })` which TransitionBackend produces
                    // for balance/nonce updates on contract accounts).
                    let code_size = match &acct_mut.code {
                        Some(CodeMut { code: Some(b) }) => b.len() as u32,
                        _ => self.state.get_code_size(addr),
                    };

                    let basic_data = pack_basic_data(0, code_size, info.nonce, info.balance);
                    let code_hash_bytes = info.code_hash.0;

                    // Stem-group write: both leaves atomically.
                    // Uses insert_stem_group_internal to keep the trie write
                    // atomic; FKV diffs are recorded separately below to match
                    // the existing pattern in update_accounts.
                    self.insert_stem_group_internal(&stem, basic_data, code_hash_bytes)
                        .map_err(|e| StateError::Other(e.to_string()))?;
                    // Record FKV diffs for the two stem-group leaves.
                    self.state
                        .record_insert(tree_key_from_stem(&stem, BASIC_DATA_LEAF_KEY), basic_data);
                    self.state.record_insert(
                        tree_key_from_stem(&stem, CODE_HASH_LEAF_KEY),
                        code_hash_bytes,
                    );

                    // Code deployment: insert chunks into trie + dual-write to code_updates.
                    if let Some(code_mut) = &acct_mut.code
                        && let Some(bytecode) = &code_mut.code
                    {
                        // Insert code chunks into the binary trie.
                        let chunks = Self::code_chunks_from_bytecode(addr, bytecode);
                        for (tree_key, chunk_value) in chunks {
                            self.state
                                .trie_insert(tree_key, chunk_value)
                                .map_err(|e| StateError::Other(e.to_string()))?;
                            // Record FKV diff for each chunk.
                            self.state.record_insert(tree_key, chunk_value);
                        }

                        // Dual-write to AccountCodes table.
                        let code = Code::from_bytecode(
                            bytes::Bytes::copy_from_slice(bytecode),
                            &NativeCrypto,
                        );
                        self.code_updates.push((info.code_hash, code));
                    }
                }
            }
        }
        Ok(())
    }

    /// Apply storage slot mutations.
    ///
    /// Zero values are deletions (EIP-7864 semantics). The `storage_keys`
    /// side-index in `BinaryTrieState` is updated via `apply_account_update`.
    fn update_storage(&mut self, addr: Address, slots: &[(H256, H256)]) -> Result<(), StateError> {
        let mut upd = AccountUpdate::new(addr);
        for (slot, value) in slots {
            upd.added_storage
                .insert(*slot, U256::from_big_endian(value.as_bytes()));
        }
        self.state
            .apply_account_update(&upd)
            .map_err(|e| StateError::Other(e.to_string()))
    }

    /// Clear all storage for an account (SELFDESTRUCT semantics).
    ///
    /// Uses the tracked `storage_keys` side-index in `BinaryTrieState` to
    /// enumerate all keys for this address and remove them from the trie.
    fn clear_storage(&mut self, addr: Address) -> Result<(), StateError> {
        let mut upd = AccountUpdate::new(addr);
        upd.removed_storage = true;
        self.state
            .apply_account_update(&upd)
            .map_err(|e| StateError::Other(e.to_string()))
    }

    /// Compute the binary trie state root.
    ///
    /// Merkelizes the trie in place and returns the root hash. Any
    /// `BinaryTrieError` is mapped to `StateError::Other`.
    fn hash(&mut self) -> Result<H256, StateError> {
        let root = self.state.state_root();
        Ok(H256(root))
    }

    /// Consume `self` and return a `MerkleOutput` containing:
    /// - The current state root.
    /// - `NodeUpdates::Binary` with serialized node diffs, deleted stems, and
    ///   inline `fkv_entries` drained from the block diffs.
    /// - `code_updates`: dual-write pairs for `AccountCodes`.
    /// - `accumulated_updates`: `None` (witness pre-computation handled by
    ///   `BinaryMerkleizer`).
    fn commit(mut self) -> Result<MerkleOutput, StateError> {
        // Compute root.
        let root = self.state.state_root();

        // Drain FKV leaf diffs accumulated during update_accounts / update_storage.
        let (_parent, fkv_entries) = self.state.take_block_diffs(root);

        // Collect dirty node diffs via `BinaryTrieState::take_trie_dirty`.
        // Pass the root hash so META_ROOT_HASH is stored alongside META_ROOT;
        // this allows readers to verify they are pinned to the correct state.
        let node_diffs = self.state.take_trie_dirty(root);

        let deleted_stems: Vec<[u8; 31]> = self.deleted_stems.into_iter().collect();

        Ok(MerkleOutput {
            root: H256(root),
            node_updates: NodeUpdates::Binary {
                node_diffs,
                deleted_stems,
                fkv_entries,
            },
            code_updates: self.code_updates,
            accumulated_updates: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::{Address, H256, U256, constants::EMPTY_KECCACK_HASH, types::AccountInfo};
    use ethrex_state_backend::{AccountMut, CodeMut, StateCommitter, StateReader};
    use rustc_hash::FxHashSet;

    use crate::key_mapping::{get_stem_for_base, tree_key_from_stem};

    fn make_addr(b: u8) -> Address {
        let mut a = [0u8; 20];
        a[19] = b;
        Address::from(a)
    }

    fn make_info(balance: u64, nonce: u64) -> AccountInfo {
        AccountInfo {
            balance: U256::from(balance),
            nonce,
            code_hash: *EMPTY_KECCACK_HASH,
        }
    }

    fn make_acct_mut(info: AccountInfo) -> AccountMut {
        AccountMut {
            account: Some(info),
            code: None,
        }
    }

    // -------------------------------------------------------------------------
    // Task 3.9 — Test 1: basic account insert and read-back via StateReader.
    // -------------------------------------------------------------------------
    #[test]
    fn test_basic_account_insert_read() {
        let mut backend = BinaryBackend::new();
        let addr = make_addr(0xAA);
        let info = make_info(1_000_000, 5);

        backend
            .update_accounts(&[addr], &[make_acct_mut(info.clone())])
            .unwrap();

        let read_back = backend.account(addr).unwrap().unwrap();
        assert_eq!(read_back.balance, info.balance);
        assert_eq!(read_back.nonce, info.nonce);
        assert_eq!(read_back.code_hash, info.code_hash);
    }

    // -------------------------------------------------------------------------
    // Task 3.9 — Test 2: storage insert, read, delete (zero-write semantics).
    // -------------------------------------------------------------------------
    #[test]
    fn test_storage_insert_read_delete() {
        let mut backend = BinaryBackend::new();
        let addr = make_addr(0xBB);
        let slot = H256::from_low_u64_be(1);
        let value = H256::from_low_u64_be(42);

        // Insert.
        backend.update_storage(addr, &[(slot, value)]).unwrap();
        assert_eq!(backend.storage(addr, slot).unwrap(), value);

        // Delete via zero write.
        backend
            .update_storage(addr, &[(slot, H256::zero())])
            .unwrap();
        assert_eq!(backend.storage(addr, slot).unwrap(), H256::zero());
    }

    // -------------------------------------------------------------------------
    // Task 3.9 — Test 3: SELFDESTRUCT produces a tombstone in deleted_stems.
    // -------------------------------------------------------------------------
    #[test]
    fn test_selfdestruct_produces_tombstone() {
        let mut backend = BinaryBackend::new();
        let addr = make_addr(0xCC);

        // First create the account.
        backend
            .update_accounts(&[addr], &[make_acct_mut(make_info(100, 1))])
            .unwrap();

        // Then self-destruct it.
        let removal = AccountMut {
            account: None,
            code: None,
        };
        backend.update_accounts(&[addr], &[removal]).unwrap();

        let expected_stem = get_stem_for_base(&addr);
        assert!(
            backend.deleted_stems.contains(&expected_stem),
            "deleted_stems must contain the account's base stem after SELFDESTRUCT"
        );
        // Account should not be readable.
        assert!(backend.account(addr).unwrap().is_none());
    }

    // -------------------------------------------------------------------------
    // Task 3.9 — Test 4: code deployment inserts chunks AND produces code_updates.
    // -------------------------------------------------------------------------
    #[test]
    fn test_code_deployment_chunks_and_code_updates() {
        let mut backend = BinaryBackend::new();
        let addr = make_addr(0xDD);

        // Bytecode: 62 bytes → 2 chunks of 31 bytes each.
        let bytecode = vec![0x5Bu8; 62]; // 62 JUMPDEST bytes
        let code = Code::from_bytecode(bytes::Bytes::copy_from_slice(&bytecode), &NativeCrypto);
        let code_hash = code.hash;

        let info = AccountInfo {
            balance: U256::from(0u64),
            nonce: 1,
            code_hash,
        };
        let acct_mut = AccountMut {
            account: Some(info),
            code: Some(CodeMut {
                code: Some(bytecode.clone()),
            }),
        };

        backend.update_accounts(&[addr], &[acct_mut]).unwrap();

        // Verify code_updates entry was added.
        assert_eq!(backend.code_updates.len(), 1);
        assert_eq!(backend.code_updates[0].0, code_hash);

        // Verify code chunk 0 is in the trie.
        let chunk0_key = get_tree_key_for_code_chunk(&addr, 0);
        assert!(
            backend.state.trie_get(chunk0_key).is_some(),
            "chunk 0 must be in the binary trie after code deployment"
        );
        let chunk1_key = get_tree_key_for_code_chunk(&addr, 1);
        assert!(
            backend.state.trie_get(chunk1_key).is_some(),
            "chunk 1 must be in the binary trie after code deployment"
        );
    }

    // -------------------------------------------------------------------------
    // Task 3.9 — Test 5: stem-group invariant — after update_accounts(Some(info)),
    // both BASIC_DATA_LEAF_KEY and CODE_HASH_LEAF_KEY are present for the stem.
    // -------------------------------------------------------------------------
    #[test]
    fn test_stem_group_invariant_both_leaves_present() {
        let mut backend = BinaryBackend::new();
        let addr = make_addr(0xEE);
        let info = make_info(500, 3);

        backend
            .update_accounts(&[addr], &[make_acct_mut(info)])
            .unwrap();

        let stem = get_stem_for_base(&addr);
        let basic_key = tree_key_from_stem(&stem, BASIC_DATA_LEAF_KEY);
        let code_key = tree_key_from_stem(&stem, CODE_HASH_LEAF_KEY);

        assert!(
            backend.state.trie_get(basic_key).is_some(),
            "BASIC_DATA_LEAF_KEY must be present in trie after update_accounts"
        );
        assert!(
            backend.state.trie_get(code_key).is_some(),
            "CODE_HASH_LEAF_KEY must be present in trie after update_accounts"
        );
    }

    // -------------------------------------------------------------------------
    // Task 3.9 — Test 6: stem-group invariant also holds after a pure balance
    // update (both leaves emitted even when only balance changes).
    // -------------------------------------------------------------------------
    #[test]
    fn test_stem_group_invariant_pure_balance_update() {
        let mut backend = BinaryBackend::new();
        let addr = make_addr(0xF0);

        // Initial insert.
        backend
            .update_accounts(&[addr], &[make_acct_mut(make_info(100, 1))])
            .unwrap();

        // Pure balance update: only balance changes, no code.
        let updated_info = AccountInfo {
            balance: U256::from(200u64),
            nonce: 1,
            code_hash: *EMPTY_KECCACK_HASH,
        };
        backend
            .update_accounts(&[addr], &[make_acct_mut(updated_info)])
            .unwrap();

        // Both leaves must still be present.
        let stem = get_stem_for_base(&addr);
        let basic_key = tree_key_from_stem(&stem, BASIC_DATA_LEAF_KEY);
        let code_key = tree_key_from_stem(&stem, CODE_HASH_LEAF_KEY);

        assert!(backend.state.trie_get(basic_key).is_some());
        assert!(backend.state.trie_get(code_key).is_some());

        // Read-back must match updated balance.
        let read = backend.account(addr).unwrap().unwrap();
        assert_eq!(read.balance, U256::from(200u64));
    }

    // -------------------------------------------------------------------------
    // Task 3.9 — Test 7: hash() returns Ok and changes after mutation.
    // -------------------------------------------------------------------------
    #[test]
    fn test_hash_changes_after_mutation() {
        let mut backend = BinaryBackend::new();

        let root_empty = backend.hash().unwrap();
        assert_eq!(root_empty, H256([0u8; 32]), "empty trie root must be zero");

        let addr = make_addr(0x01);
        backend
            .update_accounts(&[addr], &[make_acct_mut(make_info(1, 0))])
            .unwrap();

        let root_after = backend.hash().unwrap();
        assert_ne!(root_empty, root_after);
    }

    // -------------------------------------------------------------------------
    // Task 3.9 — Test 8: commit() produces NodeUpdates::Binary with deleted_stems.
    // -------------------------------------------------------------------------
    #[test]
    fn test_commit_produces_binary_node_updates() {
        let mut backend = BinaryBackend::new();
        let addr = make_addr(0x02);

        backend
            .update_accounts(&[addr], &[make_acct_mut(make_info(99, 2))])
            .unwrap();

        // Deploy code to exercise code_updates in the MerkleOutput.
        let code_addr = make_addr(0x05);
        let bytecode = vec![0x60, 0x00, 0x60, 0x00, 0xF3]; // PUSH1 0 PUSH1 0 RETURN
        let code_hash = H256::random();
        let info = AccountInfo {
            code_hash,
            balance: U256::from(100u64),
            nonce: 1,
        };
        let acct_with_code = AccountMut {
            account: Some(info),
            code: Some(CodeMut {
                code: Some(bytecode.clone()),
            }),
        };
        backend
            .update_accounts(&[code_addr], &[acct_with_code])
            .unwrap();

        // SELFDESTRUCT.
        let removal = AccountMut {
            account: None,
            code: None,
        };
        backend.update_accounts(&[addr], &[removal]).unwrap();

        let output = backend.commit().unwrap();

        match output.node_updates {
            NodeUpdates::Binary {
                node_diffs,
                deleted_stems,
                ..
            } => {
                assert!(
                    !node_diffs.is_empty(),
                    "node_diffs must not be empty after mutations"
                );
                assert_eq!(
                    deleted_stems.len(),
                    1,
                    "one deleted stem expected after SELFDESTRUCT"
                );
            }
            _ => panic!("expected NodeUpdates::Binary"),
        }

        // MerkleOutput.code_updates MUST carry the code deployment.
        assert_eq!(
            output.code_updates.len(),
            1,
            "code deployment must produce exactly one code_updates entry"
        );
        assert_eq!(
            output.code_updates[0].0, code_hash,
            "code_updates entry must be keyed by the declared code_hash"
        );
    }

    // -------------------------------------------------------------------------
    // Task 3.9 — Test 9: clear_storage removes tracked keys.
    // -------------------------------------------------------------------------
    #[test]
    fn test_clear_storage_removes_all_slots() {
        let mut backend = BinaryBackend::new();
        let addr = make_addr(0x03);

        let slot_a = H256::from_low_u64_be(1);
        let slot_b = H256::from_low_u64_be(2);

        backend
            .update_storage(addr, &[(slot_a, H256::from_low_u64_be(10))])
            .unwrap();
        backend
            .update_storage(addr, &[(slot_b, H256::from_low_u64_be(20))])
            .unwrap();

        assert_ne!(backend.storage(addr, slot_a).unwrap(), H256::zero());
        assert_ne!(backend.storage(addr, slot_b).unwrap(), H256::zero());

        backend.clear_storage(addr).unwrap();

        assert_eq!(backend.storage(addr, slot_a).unwrap(), H256::zero());
        assert_eq!(backend.storage(addr, slot_b).unwrap(), H256::zero());
    }

    // -------------------------------------------------------------------------
    // Task 3.9 — Test 10: code lookup delegates to code_reader.
    // -------------------------------------------------------------------------
    #[test]
    fn test_code_read_delegates_to_code_reader() {
        let expected: Vec<u8> = vec![0x60, 0x00, 0x56];
        let expected_clone = expected.clone();
        let addr = make_addr(0x04);
        let code_hash = H256::from_low_u64_be(0xDEAD);

        let backend = BinaryBackend {
            state: BinaryTrieState::new(),
            code_reader: Arc::new(move |h| {
                if h == code_hash {
                    Ok(Some(expected_clone.clone()))
                } else {
                    Ok(None)
                }
            }),
            provider: Arc::new(EmptyBinaryTrieProvider),
            deleted_stems: FxHashSet::default(),
            code_updates: Vec::new(),
        };

        let result = backend.code(addr, code_hash).unwrap();
        assert_eq!(result, Some(expected));
    }

    // -------------------------------------------------------------------------
    // Task 3.9 — Test 11: BackendKind::Binary is returned correctly.
    // -------------------------------------------------------------------------
    #[test]
    fn test_backend_kind_is_binary() {
        let backend = BinaryBackend::new();
        assert_eq!(backend.backend_kind(), BackendKind::Binary);
    }

    // -------------------------------------------------------------------------
    // Task 3.2 — Test: from_witness round-trip
    // -------------------------------------------------------------------------
    //
    // Generate a witness from a populated `BinaryTrieState`, then rebuild a
    // `BinaryBackend` via `from_witness` and assert: (a) the reconstructed
    // root matches the original, (b) account reads return the same values,
    // (c) storage reads return the same values.
    #[test]
    fn test_from_witness_roundtrip() {
        use std::collections::{HashMap, HashSet};

        // --- Phase 1: populate an original state with EOAs and a contract ---
        // The contract has a non-empty code_hash set via AccountInfo; we do
        // NOT pass CodeMut here — witnesses carry account basic_data +
        // code_hash leaves but NOT code chunks, so deploying a contract with
        // chunks would make the reconstructed partial trie diverge from the
        // original's full trie. See the `from_witness` doc comment for the
        // partial-reconstruction trust model.
        let mut original = BinaryBackend::new();
        let addr_eoa_a = make_addr(0xA1);
        let addr_eoa_b = make_addr(0xB2);
        let addr_contract = make_addr(0xC3);

        let contract_code_hash = H256::random();
        let contract_info_mut = AccountMut {
            account: Some(AccountInfo {
                balance: U256::from(42u64),
                nonce: 1,
                code_hash: contract_code_hash,
            }),
            code: None, // do NOT deploy chunks in the witness round-trip test
        };

        original
            .update_accounts(
                &[addr_eoa_a, addr_eoa_b, addr_contract],
                &[
                    make_acct_mut(make_info(100, 5)),
                    make_acct_mut(make_info(200, 7)),
                    contract_info_mut,
                ],
            )
            .unwrap();

        // Storage on the EOA and multiple slots on the contract account.
        let slot_eoa = H256::from_low_u64_be(42);
        let value_eoa = H256::from_low_u64_be(999);
        original
            .update_storage(addr_eoa_a, &[(slot_eoa, value_eoa)])
            .unwrap();

        let slot_c1 = H256::from_low_u64_be(1);
        let slot_c2 = H256::from_low_u64_be(2);
        let value_c1 = H256::from_low_u64_be(0xAA);
        let value_c2 = H256::from_low_u64_be(0xBB);
        original
            .update_storage(addr_contract, &[(slot_c1, value_c1), (slot_c2, value_c2)])
            .unwrap();

        let pre_state_root = original.hash().unwrap();

        // --- Phase 2: generate a witness covering all three accounts' keys ---
        let mut accessed: HashMap<Address, Vec<H256>> = HashMap::new();
        accessed.insert(addr_eoa_a, vec![slot_eoa]);
        accessed.insert(addr_eoa_b, vec![]);
        accessed.insert(addr_contract, vec![slot_c1, slot_c2]);
        let accessed_codes: HashSet<H256> = HashSet::new();
        let codes: HashMap<H256, bytes::Bytes> = HashMap::new();

        let witness = original
            .state
            .generate_witness(
                1,
                H256::from_low_u64_be(0xF00D),
                &accessed,
                &accessed_codes,
                &codes,
                vec![],
            )
            .unwrap();

        assert_eq!(
            witness.pre_state_root, pre_state_root.0,
            "witness pre_state_root must match original root"
        );
        assert_eq!(
            witness.account_proofs.len(),
            3,
            "witness must cover all three accounts"
        );
        for entry in &witness.account_proofs {
            assert!(
                entry.basic_data_proof.value.is_some() && entry.code_hash_proof.value.is_some(),
                "every witness entry must have both leaves populated (stem-group invariant); failed for {:?}",
                entry.address,
            );
        }

        // --- Phase 3: reconstruct via from_witness ---
        // NOTE: the reconstructed trie is PARTIAL (only covers witness entries);
        // its state_root() does NOT equal `pre_state_root`. We assert reads
        // round-trip for each covered entry instead.
        let reconstructed = BinaryBackend::from_witness(&witness, Arc::new(|_| Ok(None))).unwrap();

        let info_a = reconstructed.account(addr_eoa_a).unwrap().unwrap();
        assert_eq!(info_a.balance, U256::from(100u64));
        assert_eq!(info_a.nonce, 5);
        assert_eq!(info_a.code_hash, *EMPTY_KECCACK_HASH);

        let info_b = reconstructed.account(addr_eoa_b).unwrap().unwrap();
        assert_eq!(info_b.balance, U256::from(200u64));
        assert_eq!(info_b.nonce, 7);

        let info_c = reconstructed.account(addr_contract).unwrap().unwrap();
        assert_eq!(info_c.balance, U256::from(42u64));
        assert_eq!(info_c.nonce, 1);
        assert_eq!(
            info_c.code_hash, contract_code_hash,
            "contract code_hash must round-trip (non-empty hash exercises stem-group both-leaves path)"
        );

        // Storage reads round-trip for every slot in the witness.
        assert_eq!(
            reconstructed.storage(addr_eoa_a, slot_eoa).unwrap(),
            value_eoa
        );
        assert_eq!(
            reconstructed.storage(addr_contract, slot_c1).unwrap(),
            value_c1
        );
        assert_eq!(
            reconstructed.storage(addr_contract, slot_c2).unwrap(),
            value_c2
        );
    }

    // -------------------------------------------------------------------------
    // Task 3.2 — Test: from_witness rejects half-stem (one leaf present, the
    // other absent). The stem-group invariant must hold at the reconstruction
    // boundary too.
    // -------------------------------------------------------------------------
    #[test]
    fn test_from_witness_rejects_half_stem() {
        use crate::witness::{AccountWitnessEntry, BinaryTrieWitness, ProofEntry};

        // Craft a witness with only basic_data.value populated (code_hash absent)
        // — this represents a stem that was externally written with a single
        // sub-index, violating the stem-group invariant.
        let addr = make_addr(0xDE);
        let half_stem_witness = BinaryTrieWitness {
            block_number: 1,
            block_hash: H256::zero(),
            pre_state_root: [0u8; 32], // arbitrary; we expect rejection before root check
            account_proofs: vec![AccountWitnessEntry {
                address: addr,
                balance: U256::from(1u64),
                nonce: 1,
                code_hash: *EMPTY_KECCACK_HASH,
                basic_data_proof: ProofEntry {
                    siblings: vec![],
                    stem_depth: 0,
                    value: Some([0x11; 32]), // basic_data present
                },
                code_hash_proof: ProofEntry {
                    siblings: vec![],
                    stem_depth: 0,
                    value: None, // code_hash absent — half-stem
                },
            }],
            storage_proofs: vec![],
            codes: vec![],
            block_headers: vec![],
        };

        let result = BinaryBackend::from_witness(&half_stem_witness, Arc::new(|_| Ok(None)));
        match result {
            Err(BinaryTrieError::InvalidWitness) => {}
            Err(other) => panic!("expected InvalidWitness for half-stem, got {other:?}"),
            Ok(_) => panic!("reconstruction must fail on half-stem witness"),
        }

        // Symmetric case: code_hash present, basic_data absent.
        let half_stem_witness_rev = BinaryTrieWitness {
            block_number: 1,
            block_hash: H256::zero(),
            pre_state_root: [0u8; 32],
            account_proofs: vec![AccountWitnessEntry {
                address: addr,
                balance: U256::zero(),
                nonce: 0,
                code_hash: *EMPTY_KECCACK_HASH,
                basic_data_proof: ProofEntry {
                    siblings: vec![],
                    stem_depth: 0,
                    value: None,
                },
                code_hash_proof: ProofEntry {
                    siblings: vec![],
                    stem_depth: 0,
                    value: Some([0x22; 32]),
                },
            }],
            storage_proofs: vec![],
            codes: vec![],
            block_headers: vec![],
        };
        let result_rev =
            BinaryBackend::from_witness(&half_stem_witness_rev, Arc::new(|_| Ok(None)));
        match result_rev {
            Err(BinaryTrieError::InvalidWitness) => {}
            Err(other) => panic!("expected InvalidWitness for reversed half-stem, got {other:?}"),
            Ok(_) => panic!("reconstruction must fail on reversed half-stem witness"),
        }
    }
}
