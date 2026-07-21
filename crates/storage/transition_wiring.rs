//! Transition-specific wiring: `TransitionBackend`, persistence helpers, and
//! `Store` factory methods for constructing the MPT→binary transition state.
//!
//! # Transition semantics (EIP-7612 pure overlay)
//!
//! After the switch block:
//! - **Reads**: overlay (binary) first; if not found, fall through to base (MPT).
//! - **Writes**: overlay only, never base.
//! - **CoW on first touch**: the first post-switch write to an MPT-resident
//!   account atomically writes all four `BASIC_DATA` sub-leaves plus `CODE_HASH`
//!   into the overlay so that all subsequent reads are branch-free (an account
//!   is either fully in overlay or fully in base; no partial-stem state).
//!
//! # Tombstones
//!
//! A SELFDESTRUCT post-switch writes a tombstone entry to the overlay's
//! `deleted_stems` set (and eventually to disk). A subsequent read for a
//! tombstoned stem returns `None` / `H256::zero()` without falling through to
//! the MPT base. A re-creation after selfdestruct clears the tombstone and
//! treats the account as a fresh overlay entry.
//!
//! # Read-only base invariant
//!
//! `TransitionBackend` never calls `StateCommitter` methods on `base`. This is
//! enforced at the type level: the only `&mut base` access is the initial
//! construction (moving a `MptBackend` by value) and there is none elsewhere.

use ethrex_binary_trie::{
    BinaryBackend,
    key_mapping::{BASIC_DATA_LEAF_KEY, CODE_HASH_LEAF_KEY, get_stem_for_base, pack_basic_data},
    state::BinaryTrieState,
};
use ethrex_common::{H256, constants::EMPTY_KECCACK_HASH, types::AccountInfo};
use ethrex_state_backend::{
    AccountMut, CodeReader, MerkleOutput, StateCommitter, StateError, StateReader,
};
use ethrex_trie::MptBackend;

use crate::{
    Store,
    api::tables::{
        BINARY_STORAGE_KEYS, BINARY_TRIE_NODES, MISC_VALUES, STATE_BACKEND_FORMAT_KEY,
        TRANSITION_BINARY_ROOT_KEY, TRANSITION_MPT_FROZEN_ROOT_KEY, TRANSITION_SWITCH_BLOCK_KEY,
    },
    binary_wiring::{StorageTrieBackend, StoreBinaryTrieProvider},
    error::StoreError,
    state_backend::StateBackend,
};
use ethrex_common::Address;
use ethrex_crypto::NativeCrypto;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// TransitionBackend
// ---------------------------------------------------------------------------

/// Composite state backend for the MPT→binary transition.
///
/// `base` is the frozen MPT at the switch block; it is **never mutated**.
/// `overlay` is the binary trie receiving all post-switch writes.
///
/// Read semantics:
/// - Tombstoned stem → `None` / zero (no MPT fallthrough).
/// - Stem present in overlay → read from overlay.
/// - Otherwise → fall through to base.
///
/// Write semantics (CoW on first touch):
/// - If stem is tombstoned in overlay → clear tombstone; treat as fresh.
/// - If stem is not yet in overlay → CoW: copy full stem from base atomically.
/// - Then apply the update to the overlay.
pub struct TransitionBackend {
    /// Frozen MPT state at the switch block. Read-only.
    pub(crate) base: MptBackend,
    /// Binary trie overlay receiving all post-switch writes.
    pub(crate) overlay: BinaryBackend,
    /// The first block number that writes to the binary overlay.
    /// Retained for Phase 7 activation and operational logging.
    #[allow(dead_code)]
    pub(crate) switch_block: u64,
    /// MPT state root at the switch block (post-state of `switch_block - 1`).
    /// Retained for Phase 7 activation and RPC proof responses.
    #[allow(dead_code)]
    pub(crate) frozen_mpt_root: H256,
    /// Code reader — shared between base and overlay reads.
    code_reader: CodeReader,
}

impl TransitionBackend {
    /// Create a new `TransitionBackend` from existing backends.
    ///
    /// `base` must not be mutated after construction. `overlay` should be an
    /// empty (or partially-populated) binary backend for post-switch writes.
    pub fn new(
        base: MptBackend,
        overlay: BinaryBackend,
        switch_block: u64,
        frozen_mpt_root: H256,
        code_reader: CodeReader,
    ) -> Self {
        Self {
            base,
            overlay,
            switch_block,
            frozen_mpt_root,
            code_reader,
        }
    }

    /// Derive the binary trie stem for `addr` (BLAKE3-based, per EIP-7864).
    fn stem(addr: &Address) -> [u8; 31] {
        get_stem_for_base(addr)
    }

    /// Perform a CoW pull of an MPT-resident account into the binary overlay.
    ///
    /// Reads the full account state from `base` and inserts both
    /// `BASIC_DATA_LEAF_KEY` and `CODE_HASH_LEAF_KEY` atomically into `overlay`.
    /// After this call the account is "in overlay" and subsequent reads never
    /// touch `base` for this stem.
    ///
    /// `code_size` is derived from the bytecode length: if `code_hash` is
    /// the empty keccak hash, `code_size = 0`; otherwise we load the bytecode
    /// via `code_reader` and use its length. This is correct per EIP-7864 where
    /// `code_size` is a property of the packed `BASIC_DATA` leaf.
    fn cow_pull_from_base(&mut self, addr: &Address, stem: &[u8; 31]) -> Result<(), StateError> {
        let account_info = self.base.account(*addr)?;
        let (nonce, balance, code_hash) = match account_info {
            Some(info) => (info.nonce, info.balance, info.code_hash),
            // Brand-new account not in MPT: use zero fields. The CoW still writes
            // the full stem group atomically to establish the invariant.
            None => (0u64, ethrex_common::U256::zero(), *EMPTY_KECCACK_HASH),
        };

        // Derive code_size from bytecode length. `code_size` is stored in the
        // binary trie's BASIC_DATA leaf and is NOT present in AccountInfo (which
        // is MPT-centric). We must derive it by reading the bytecode.
        let code_size: u32 = if code_hash == *EMPTY_KECCACK_HASH {
            0
        } else {
            match (self.code_reader)(code_hash)? {
                Some(bytecode) => bytecode.len() as u32,
                None => {
                    tracing::warn!(
                        "CoW: bytecode missing for code_hash {:?} on account {:?}; \
                         writing code_size=0 — likely indicates a corrupted state",
                        code_hash,
                        addr
                    );
                    0
                }
            }
        };

        let packed = pack_basic_data(0, code_size, nonce, balance);
        self.overlay.insert_stem_group(
            stem,
            &[
                (BASIC_DATA_LEAF_KEY, packed),
                (CODE_HASH_LEAF_KEY, code_hash.0),
            ],
        )
    }
}

// ---------------------------------------------------------------------------
// StateReader for TransitionBackend
// ---------------------------------------------------------------------------

impl StateReader for TransitionBackend {
    /// Read an account from the transition state.
    ///
    /// Path:
    /// 1. Tombstone check → `None` (no MPT fallthrough).
    /// 2. Overlay has basic_data → reconstruct from overlay.
    /// 3. Fall through to `base.account(addr)`.
    fn account(&self, addr: Address) -> Result<Option<AccountInfo>, StateError> {
        let stem = Self::stem(&addr);

        if self.overlay.stem_is_tombstoned(&stem)? {
            return Ok(None);
        }

        if self.overlay.stem_has_basic_data(&stem)? {
            // By the stem-group invariant, CODE_HASH is also present.
            return self.overlay.account(addr);
        }

        // Fall through to frozen MPT base.
        self.base.account(addr)
    }

    /// Read a storage slot from the transition state.
    ///
    /// Path:
    /// 1. Overlay has a non-zero value → return it.
    /// 2. Stem is tombstoned → return zero (account was selfdestructed; all
    ///    storage wiped).
    /// 3. Slot has any record in overlay (including explicit zero marker from
    ///    a post-switch SSTORE 0) → return zero.
    /// 4. Fall through to `base.storage(addr, slot)`.
    fn storage(&self, addr: Address, slot: H256) -> Result<H256, StateError> {
        let stem = Self::stem(&addr);

        // Check overlay first: if there's a non-zero value, return it.
        let overlay_val = self.overlay.storage(addr, slot)?;
        if overlay_val != H256::zero() {
            return Ok(overlay_val);
        }

        // Zero from overlay is ambiguous: "not yet written" vs "explicitly zeroed".
        // Use tombstone to disambiguate: tombstone → account selfdestructed, all
        // storage is gone; no MPT fallthrough.
        if self.overlay.stem_is_tombstoned(&stem)? {
            return Ok(H256::zero());
        }

        // Distinguish "explicitly zeroed in overlay" from "absent in overlay".
        // A post-switch SSTORE 0 to a slot that held a non-zero pre-switch value
        // must hide the MPT value, not fall through. The binary trie deletes the
        // leaf on zero-write (EIP-7864 standalone semantics), so we record a zero
        // marker in the FKV and check for any FKV record here.
        if self.overlay.slot_is_in_overlay(addr, slot)? {
            return Ok(H256::zero());
        }

        self.base.storage(addr, slot)
    }

    /// Read code by hash.
    ///
    /// Single path: `code_reader` (the legacy `AccountCodes` table). Pre-switch
    /// and post-switch codes are both available there. No chunk reconstruction.
    fn code(&self, _addr: Address, code_hash: H256) -> Result<Option<Vec<u8>>, StateError> {
        (self.code_reader)(code_hash)
    }
}

// ---------------------------------------------------------------------------
// StateCommitter for TransitionBackend
// ---------------------------------------------------------------------------

impl StateCommitter for TransitionBackend {
    /// Apply account mutations with atomic CoW on first touch.
    ///
    /// For each `(addr, acct_mut)` pair:
    /// - `None` (SELFDESTRUCT): add tombstone to overlay; skip CoW.
    /// - `Some(info)`: CoW if stem not yet in overlay (pulls full stem from
    ///   base atomically), then apply the update via `overlay.update_accounts`.
    fn update_accounts(
        &mut self,
        addrs: &[Address],
        muts: &[AccountMut],
    ) -> Result<(), StateError> {
        for (addr, acct_mut) in addrs.iter().zip(muts.iter()) {
            let stem = Self::stem(addr);

            match &acct_mut.account {
                None => {
                    // SELFDESTRUCT: write tombstone to overlay and clear overlay
                    // storage. The tombstone hides the MPT base for future reads.
                    // No CoW needed: we're deleting the account, not creating it.
                    self.overlay.update_accounts(
                        &[*addr],
                        &[AccountMut {
                            account: None,
                            code: None,
                        }],
                    )?;
                }
                Some(_info) => {
                    if self.overlay.stem_is_tombstoned(&stem)? {
                        // Post-selfdestruct re-creation: clear tombstone by
                        // delegating directly to overlay (which removes from
                        // deleted_stems when a new account is written). No CoW
                        // from base — the account was deleted; start fresh.
                        self.overlay
                            .update_accounts(&[*addr], std::slice::from_ref(acct_mut))?;
                    } else if !self.overlay.stem_has_basic_data(&stem)? {
                        // First write to an MPT-resident account: CoW pull.
                        self.cow_pull_from_base(addr, &stem)?;
                        // Apply the actual update on top of the CoW'd stem.
                        self.overlay
                            .update_accounts(&[*addr], std::slice::from_ref(acct_mut))?;
                    } else {
                        // Stem already in overlay: apply update directly.
                        self.overlay
                            .update_accounts(&[*addr], std::slice::from_ref(acct_mut))?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Apply storage slot mutations.
    ///
    /// Delegates directly to the overlay. Storage slots fall through to base on
    /// read if not present in overlay; explicit CoW is not needed for storage
    /// (only accounts need the atomic stem-group invariant).
    fn update_storage(&mut self, addr: Address, slots: &[(H256, H256)]) -> Result<(), StateError> {
        self.overlay.update_storage(addr, slots)
    }

    /// Clear all storage for an account (SELFDESTRUCT or `removed_storage`).
    ///
    /// Writes a tombstone into the overlay's `deleted_stems` set so that the
    /// MPT base's storage is permanently hidden for subsequent reads, then
    /// clears the overlay's own storage index for this address.
    ///
    /// The tombstone write must come first: `storage()` checks tombstone before
    /// falling through to base. Without it, pre-switch storage would remain
    /// visible on reads even after `clear_storage`.
    fn clear_storage(&mut self, addr: Address) -> Result<(), StateError> {
        // Write tombstone so MPT base storage is hidden.
        let stem = Self::stem(&addr);
        self.overlay.tombstone_stem(&stem);
        // Clear overlay's own storage index.
        self.overlay.clear_storage(addr)
    }

    /// Compute the binary trie overlay root.
    ///
    /// We do NOT compose base + overlay into a single root; state-root
    /// validation is disabled post-switch (the block header carries the MPT
    /// root). Returns the overlay's root as-is.
    fn hash(&mut self) -> Result<H256, StateError> {
        self.overlay.hash()
    }

    /// Commit the overlay and return its `MerkleOutput`.
    ///
    /// Only the overlay is committed; the base (frozen MPT) is unchanged.
    fn commit(self) -> Result<MerkleOutput, StateError> {
        self.overlay.commit()
    }
}

// ---------------------------------------------------------------------------
// impl Store: transition factory methods
// ---------------------------------------------------------------------------

impl Store {
    /// Create a [`StateBackend::Transition`] anchored at the frozen MPT root
    /// and the **live** binary head root.
    ///
    /// The MPT side is treated as read-only by construction (no `&mut base` is
    /// exposed); writes go to the binary overlay only.
    ///
    /// - `mpt_root`: the frozen MPT root at the switch block.
    /// - `switch_block`: block number of the first binary-overlay block.
    ///
    /// The binary overlay is anchored at `Store::current_binary_root()` (the
    /// live, in-memory head advanced per-block by `apply_trie_updates`). The
    /// overlay is opened via [`CacheAwareTrieBackend`] so the in-memory
    /// [`BinaryTrieState`] traverses the **live** structure (cache layers +
    /// disk), not the disk-flushed root which lags by up to 128 layers. This
    /// matches MPT's `MptTrieWrapper(state_root, trie_cache, db, last_written)`.
    ///
    /// The `binary_root` field of `transition_metadata` is no longer consulted;
    /// it is kept on disk for backward-compatibility of the format but is
    /// vestigial — the live root is the source of truth.
    pub fn new_transition_state_reader(
        &self,
        switch_block: u64,
        mpt_root: H256,
    ) -> Result<StateBackend, StoreError> {
        use crate::binary_wiring::CacheAwareTrieBackend;
        use crate::mpt_wiring::StoreTrieProvider;

        // Open the frozen MPT backend at `mpt_root`.
        let state_trie = self.open_state_trie(mpt_root)?;
        let mpt_provider = Arc::new(StoreTrieProvider {
            store: self.clone(),
            parent_state_root: mpt_root,
        }) as Arc<dyn ethrex_trie::TrieProvider>;
        let code_reader = self.make_code_reader();
        let base = MptBackend::new_with_db(
            state_trie,
            Arc::new(NativeCrypto),
            mpt_provider,
            code_reader.clone(),
        );

        // Open the binary overlay backend, anchored at the live head root via
        // CacheAwareTrieBackend.
        //
        // BinaryTrieState::open reads META_ROOT through the wrapper; the cache
        // layer at current_binary_root contains an updated META_ROOT entry
        // pointing at the live root NodeId. If no commits have happened yet
        // (fresh activation, current_binary_root == EMPTY_BINARY_ROOT), the
        // cache lookup returns None, the disk lookup also returns None, and
        // the trie is opened empty — same as the previous "fresh activation"
        // branch.
        let binary_provider = Arc::new(StoreBinaryTrieProvider {
            store: self.clone(),
        }) as Arc<dyn ethrex_binary_trie::BinaryTrieProvider>;
        let trie_backend = Arc::new(CacheAwareTrieBackend {
            store: self.clone(),
            inner: StorageTrieBackend {
                store: self.clone(),
            },
        });
        let binary_state =
            BinaryTrieState::open(trie_backend, BINARY_TRIE_NODES, BINARY_STORAGE_KEYS)
                .map_err(|e| StoreError::Custom(e.to_string()))?;
        let overlay = BinaryBackend::from_state(binary_state, binary_provider, code_reader.clone());

        let transition = TransitionBackend::new(base, overlay, switch_block, mpt_root, code_reader);
        Ok(StateBackend::Transition(Box::new(transition)))
    }

    /// Persist the three transition metadata keys plus the format byte atomically.
    ///
    /// Writes:
    /// - `STATE_BACKEND_FORMAT_KEY` → `2` (byte for `BackendKind::Transition`).
    /// - `TRANSITION_SWITCH_BLOCK_KEY` → `switch_block` as 8-byte big-endian u64.
    /// - `TRANSITION_MPT_FROZEN_ROOT_KEY` → `mpt_root` as 32 raw bytes.
    /// - `TRANSITION_BINARY_ROOT_KEY` → `binary_root` as 32 raw bytes.
    ///
    /// All four writes are in one `begin_write` / `commit` block. Either all
    /// land or none do.
    pub fn persist_transition_metadata(
        &self,
        switch_block: u64,
        mpt_root: H256,
        binary_root: H256,
    ) -> Result<(), StoreError> {
        use crate::store::backend_kind_to_byte;
        use ethrex_state_backend::BackendKind;

        let mut tx = self.backend.begin_write()?;
        tx.put(
            MISC_VALUES,
            STATE_BACKEND_FORMAT_KEY,
            &[backend_kind_to_byte(BackendKind::Transition)],
        )?;
        tx.put(
            MISC_VALUES,
            TRANSITION_SWITCH_BLOCK_KEY,
            &switch_block.to_be_bytes(),
        )?;
        tx.put(
            MISC_VALUES,
            TRANSITION_MPT_FROZEN_ROOT_KEY,
            mpt_root.as_bytes(),
        )?;
        tx.put(
            MISC_VALUES,
            TRANSITION_BINARY_ROOT_KEY,
            binary_root.as_bytes(),
        )?;
        tx.commit()?;
        // Disk write succeeded; update the in-memory RwLock so the store
        // immediately reflects the new metadata without a restart.
        // Disk-first: if commit() had errored above, we would have returned
        // early without touching the in-memory value.
        *self
            .transition_metadata
            .write()
            .expect("transition_metadata RwLock poisoned") =
            Some((switch_block, mpt_root, binary_root));
        Ok(())
    }

    /// Load the three transition metadata keys from `MISC_VALUES`.
    ///
    /// Semantics:
    /// - All three absent → `Ok(None)`. The store has not been activated for transition yet.
    /// - All three present and well-formed → `Ok(Some((switch_block, mpt_root, binary_root)))`.
    /// - Anything else (partial presence, or any key present but with unexpected byte length)
    ///   → `Err(StoreError::Custom(...))`. This indicates DB corruption.
    pub fn load_transition_metadata(&self) -> Result<Option<(u64, H256, H256)>, StoreError> {
        let tx = self.backend.begin_read()?;

        let switch_block_bytes = tx.get(MISC_VALUES, TRANSITION_SWITCH_BLOCK_KEY)?;
        let mpt_root_bytes = tx.get(MISC_VALUES, TRANSITION_MPT_FROZEN_ROOT_KEY)?;
        let binary_root_bytes = tx.get(MISC_VALUES, TRANSITION_BINARY_ROOT_KEY)?;

        match (switch_block_bytes, mpt_root_bytes, binary_root_bytes) {
            (None, None, None) => Ok(None),
            (Some(sb), Some(mr), Some(br)) => {
                let switch_block = decode_u64_be(&sb).ok_or_else(|| {
                    StoreError::Custom(format!(
                        "TRANSITION_SWITCH_BLOCK_KEY has unexpected length {} (expected 8)",
                        sb.len()
                    ))
                })?;
                let mpt_root = decode_h256(&mr).ok_or_else(|| {
                    StoreError::Custom(format!(
                        "TRANSITION_MPT_FROZEN_ROOT_KEY has unexpected length {} (expected 32)",
                        mr.len()
                    ))
                })?;
                let binary_root = decode_h256(&br).ok_or_else(|| {
                    StoreError::Custom(format!(
                        "TRANSITION_BINARY_ROOT_KEY has unexpected length {} (expected 32)",
                        br.len()
                    ))
                })?;
                Ok(Some((switch_block, mpt_root, binary_root)))
            }
            // Partial presence: some keys written but not all — indicates
            // an incomplete activation write or DB corruption.
            _ => Err(StoreError::Custom(
                "partial/corrupt transition metadata: some keys present, others absent".to_string(),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Decode helpers
// ---------------------------------------------------------------------------

fn decode_u64_be(bytes: &[u8]) -> Option<u64> {
    if bytes.len() == 8 {
        Some(u64::from_be_bytes(
            bytes.try_into().expect("length checked above"),
        ))
    } else {
        None
    }
}

fn decode_h256(bytes: &[u8]) -> Option<H256> {
    if bytes.len() == 32 {
        Some(H256::from_slice(bytes))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Unit tests (Tasks 6.10, 6.11, 6.12)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Store,
        api::tables::{ACCOUNT_CODE_METADATA, ACCOUNT_CODES},
        store::{EngineType, encode_code},
    };
    use ethrex_common::{
        Address, H256, U256,
        constants::EMPTY_KECCACK_HASH,
        types::{AccountInfo, Code},
    };
    use ethrex_crypto::NativeCrypto;
    use ethrex_state_backend::{AccountMut, BackendKind, StateCommitter, StateReader};

    // -------------------------------------------------------------------------
    // Test helpers
    // -------------------------------------------------------------------------

    fn make_addr(b: u8) -> Address {
        let mut a = [0u8; 20];
        a[19] = b;
        Address::from(a)
    }

    fn make_acct_mut(info: AccountInfo) -> AccountMut {
        AccountMut {
            account: Some(info),
            code: None,
        }
    }

    /// Construct a `Store` with a seeded MPT account, then return
    /// (store, switch_block_root, addr) so tests can build a TransitionBackend.
    fn setup_mpt_with_account(addr: Address, nonce: u64, balance: u64) -> (Store, H256, Address) {
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();

        // Write the account into MPT via the state writer.
        let mut backend = store.new_state_writer().unwrap();
        backend
            .update_accounts(
                &[addr],
                &[make_acct_mut(AccountInfo {
                    balance: U256::from(balance),
                    nonce,
                    code_hash: *EMPTY_KECCACK_HASH,
                })],
            )
            .unwrap();
        let output = backend.commit().unwrap();
        let mpt_root = output.root;

        // Write the trie nodes to the store so the MPT reader can open the trie.
        store
            .write_node_updates_direct(output.node_updates)
            .unwrap();

        (store, mpt_root, addr)
    }

    // -------------------------------------------------------------------------
    // Task 6.10: CoW invariant
    //
    // Pre-touch account A in MPT with nonce=5, balance=10. Post-switch, call
    // update_accounts with only a balance change. Assert that the overlay stem
    // contains all four BASIC_DATA fields (including nonce=5 from MPT) plus
    // CODE_HASH preserved.
    // -------------------------------------------------------------------------
    #[test]
    fn test_cow_pulls_full_stem_from_mpt() {
        let addr = make_addr(0xA1);
        let (store, mpt_root, _) = setup_mpt_with_account(addr, 5, 10);

        // Build a TransitionBackend over the store.
        let code_reader = store.make_code_reader();
        let base = {
            let state_trie = store.open_state_trie(mpt_root).unwrap();
            let provider = Arc::new(crate::mpt_wiring::StoreTrieProvider {
                store: store.clone(),
                parent_state_root: mpt_root,
            }) as Arc<dyn ethrex_trie::TrieProvider>;
            MptBackend::new_with_db(
                state_trie,
                Arc::new(NativeCrypto),
                provider,
                code_reader.clone(),
            )
        };
        let overlay = BinaryBackend::new();
        let mut tb = TransitionBackend::new(base, overlay, 1, mpt_root, code_reader);

        let stem = get_stem_for_base(&addr);

        // Stem must NOT be in overlay before the update.
        assert!(
            !tb.overlay.stem_has_basic_data(&stem).unwrap(),
            "stem must not be in overlay before first update"
        );

        // Apply a balance-only update (nonce unchanged from MPT value 5).
        let new_info = AccountInfo {
            balance: U256::from(999u64),
            nonce: 5,
            code_hash: *EMPTY_KECCACK_HASH,
        };
        tb.update_accounts(&[addr], &[make_acct_mut(new_info)])
            .unwrap();

        // After the update the stem must be in the overlay (CoW + apply happened).
        assert!(
            tb.overlay.stem_has_basic_data(&stem).unwrap(),
            "stem must be in overlay after first update (CoW + apply)"
        );

        // Read account from the overlay directly to verify CoW pulled the full
        // stem. `overlay.account(addr)` returns Some iff both BASIC_DATA_LEAF_KEY
        // and CODE_HASH_LEAF_KEY are present (debug_assert! enforces stem-group
        // invariant inside BinaryBackend::account in debug builds).
        let overlay_info = tb
            .overlay
            .account(addr)
            .expect("overlay.account must succeed")
            .expect("overlay must have the account after CoW");

        // Nonce must be the original value pulled from MPT (not 0 or any default).
        assert_eq!(
            overlay_info.nonce, 5,
            "nonce must be preserved from MPT during CoW"
        );
        // Balance must reflect the post-switch update, not the original MPT value.
        assert_eq!(
            overlay_info.balance,
            U256::from(999u64),
            "balance must reflect the post-switch update"
        );
        // CODE_HASH must be EMPTY (the account had no code in MPT).
        assert_eq!(
            overlay_info.code_hash, *EMPTY_KECCACK_HASH,
            "CODE_HASH must be preserved from MPT"
        );

        // Verify read through TransitionBackend returns the updated values
        // (reads from overlay, not base).
        let tb_info = tb.account(addr).unwrap().unwrap();
        assert_eq!(tb_info.balance, U256::from(999u64));
        assert_eq!(tb_info.nonce, 5);
    }

    // -------------------------------------------------------------------------
    // Task 6.11: round-trip + tombstone cascade + read-only-base invariant
    // -------------------------------------------------------------------------
    #[test]
    fn test_transition_round_trip_tombstone_and_readonly_base() {
        let addr_a = make_addr(0xA2);
        let addr_b = make_addr(0xB2);
        let (store, mpt_root, _) = setup_mpt_with_account(addr_a, 1, 100);

        let code_reader = store.make_code_reader();

        // Helper: build a fresh TransitionBackend for each sub-test.
        let make_tb = || {
            let state_trie = store.open_state_trie(mpt_root).unwrap();
            let provider = Arc::new(crate::mpt_wiring::StoreTrieProvider {
                store: store.clone(),
                parent_state_root: mpt_root,
            }) as Arc<dyn ethrex_trie::TrieProvider>;
            let base = MptBackend::new_with_db(
                state_trie,
                Arc::new(NativeCrypto),
                provider,
                code_reader.clone(),
            );
            TransitionBackend::new(base, BinaryBackend::new(), 1, mpt_root, code_reader.clone())
        };

        // --- Sub-test 1: pre-write reads from base ---
        {
            let tb = make_tb();
            let info = tb.account(addr_a).unwrap().unwrap();
            assert_eq!(info.nonce, 1, "pre-write read must come from base MPT");
            assert_eq!(info.balance, U256::from(100u64));
        }

        // --- Sub-test 2: post-write reads from overlay ---
        {
            let mut tb = make_tb();
            tb.update_accounts(
                &[addr_a],
                &[make_acct_mut(AccountInfo {
                    balance: U256::from(200u64),
                    nonce: 1,
                    code_hash: *EMPTY_KECCACK_HASH,
                })],
            )
            .unwrap();
            let info = tb.account(addr_a).unwrap().unwrap();
            assert_eq!(
                info.balance,
                U256::from(200u64),
                "post-write read must come from overlay"
            );
        }

        // --- Sub-test 3: tombstone cascade ---
        // Selfdestruct addr_a → tombstone in overlay hides MPT entry.
        // Re-create addr_a → tombstone cleared, fresh overlay entry used.
        {
            let mut tb = make_tb();
            // addr_a exists in MPT. Selfdestruct it.
            tb.update_accounts(
                &[addr_a],
                &[AccountMut {
                    account: None,
                    code: None,
                }],
            )
            .unwrap();
            // Must not be readable (tombstone hides MPT base).
            assert!(
                tb.account(addr_a).unwrap().is_none(),
                "tombstoned account must return None"
            );

            // Re-create addr_a.
            let fresh_info = AccountInfo {
                balance: U256::from(777u64),
                nonce: 99,
                code_hash: *EMPTY_KECCACK_HASH,
            };
            tb.update_accounts(&[addr_a], &[make_acct_mut(fresh_info)])
                .unwrap();

            // Now it's readable and has the new values.
            let read = tb.account(addr_a).unwrap().unwrap();
            assert_eq!(
                read.balance,
                U256::from(777u64),
                "re-created account must have new balance"
            );
            assert_eq!(read.nonce, 99, "re-created account must have new nonce");
        }

        // --- Sub-test 4: read-only base invariant ---
        // Snapshot the base account state before any mutations. After a series
        // of update_accounts / update_storage / clear_storage on the
        // TransitionBackend, the base MPT account must still return the same
        // values as at construction time.
        {
            let mut tb = make_tb();

            // Capture the base state for addr_a before any mutations.
            let base_info_before = tb.base.account(addr_a).unwrap();

            // Mutation sequence.
            tb.update_accounts(
                &[addr_a],
                &[make_acct_mut(AccountInfo {
                    balance: U256::from(42u64),
                    nonce: 1,
                    code_hash: *EMPTY_KECCACK_HASH,
                })],
            )
            .unwrap();
            tb.update_storage(
                addr_b,
                &[(H256::from_low_u64_be(1), H256::from_low_u64_be(2))],
            )
            .unwrap();
            tb.clear_storage(addr_a).unwrap();
            tb.update_accounts(
                &[addr_b],
                &[AccountMut {
                    account: None,
                    code: None,
                }],
            )
            .unwrap();

            // Base must still return the same values for addr_a (no mutation
            // happened via the base reference — read-only invariant holds).
            let base_info_after = tb.base.account(addr_a).unwrap();
            assert_eq!(
                base_info_before, base_info_after,
                "base MPT account must not change after TransitionBackend mutations"
            );
        }
    }

    // -------------------------------------------------------------------------
    // Task 6.10 (nonempty-code variant): CoW pulls code_size from code_reader
    //
    // Pre-touch a contract account in MPT with real bytecode. Post-switch, do a
    // balance-only update. Assert that the overlay's BASIC_DATA leaf has
    // code_size == bytecode.len() (not 0), proving the CoW code_reader path ran.
    // -------------------------------------------------------------------------
    #[test]
    fn test_cow_pulls_full_stem_with_nonempty_code() {
        let addr = make_addr(0xA2);

        // --- Build a Store with Mpt backend ---
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();

        // Bytecode: 62 JUMPDEST bytes (same constant as BinaryBackend tests).
        let bytecode: Vec<u8> = vec![0x5Bu8; 62];
        let code = Code::from_bytecode(bytes::Bytes::copy_from_slice(&bytecode), &NativeCrypto);
        let code_hash = code.hash;

        // Write bytecode into the ACCOUNT_CODES table so code_reader can find it.
        {
            let encoded = encode_code(&code);
            let metadata_buf = (bytecode.len() as u64).to_be_bytes();
            store
                .write(ACCOUNT_CODES, code_hash.0.to_vec(), encoded)
                .unwrap();
            store
                .write(
                    ACCOUNT_CODE_METADATA,
                    code_hash.0.to_vec(),
                    metadata_buf.to_vec(),
                )
                .unwrap();
        }

        // Write the contract account into MPT.
        let mut backend = store.new_state_writer().unwrap();
        backend
            .update_accounts(
                &[addr],
                &[make_acct_mut(AccountInfo {
                    balance: U256::from(0u64),
                    nonce: 1,
                    code_hash,
                })],
            )
            .unwrap();
        let output = backend.commit().unwrap();
        let mpt_root = output.root;
        store
            .write_node_updates_direct(output.node_updates)
            .unwrap();

        // --- Build TransitionBackend ---
        let code_reader = store.make_code_reader();
        let base = {
            let state_trie = store.open_state_trie(mpt_root).unwrap();
            let provider = Arc::new(crate::mpt_wiring::StoreTrieProvider {
                store: store.clone(),
                parent_state_root: mpt_root,
            }) as Arc<dyn ethrex_trie::TrieProvider>;
            MptBackend::new_with_db(
                state_trie,
                Arc::new(NativeCrypto),
                provider,
                code_reader.clone(),
            )
        };
        let overlay = BinaryBackend::new_with_db(
            Arc::new(crate::binary_wiring::StoreBinaryTrieProvider {
                store: store.clone(),
            }) as Arc<dyn ethrex_binary_trie::BinaryTrieProvider>,
            code_reader.clone(),
        );
        let mut tb = TransitionBackend::new(base, overlay, 1, mpt_root, code_reader);

        // --- Trigger CoW with a balance-only update ---
        let new_info = AccountInfo {
            balance: U256::from(100u64),
            nonce: 1,
            code_hash,
        };
        tb.update_accounts(&[addr], &[make_acct_mut(new_info)])
            .unwrap();

        // The overlay must now have the account's BASIC_DATA leaf.
        let stem = get_stem_for_base(&addr);
        assert!(
            tb.overlay.stem_has_basic_data(&stem).unwrap(),
            "stem must be in overlay after CoW"
        );

        // Assert code_size == bytecode.len() (the CoW code_reader path executed).
        // If code_reader returned None and silently fell back to 0, this would fail.
        let overlay_code_size = tb.overlay.get_code_size(&addr);
        assert_eq!(
            overlay_code_size,
            bytecode.len() as u32,
            "code_size in overlay BASIC_DATA must equal bytecode length after CoW"
        );

        // Also assert code_hash is preserved.
        let overlay_info = tb.overlay.account(addr).unwrap().unwrap();
        assert_eq!(
            overlay_info.code_hash, code_hash,
            "code_hash must be preserved from MPT during CoW"
        );
    }

    // -------------------------------------------------------------------------
    // Task 6.11 (clear_storage tombstone): clear_storage writes tombstone so
    // MPT base storage is hidden, even without a prior account removal.
    //
    // Regression for: clear_storage only delegated to overlay (no tombstone),
    // so base storage remained visible.
    // -------------------------------------------------------------------------
    #[test]
    fn test_clear_storage_writes_tombstone_hiding_base() {
        let addr_a = make_addr(0xD1);
        let slot = H256::from_low_u64_be(42);
        let value = H256::from_low_u64_be(42);

        // Build a store with addr_a having storage slot S = 42 in MPT.
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();
        let mut backend = store.new_state_writer().unwrap();
        backend
            .update_accounts(
                &[addr_a],
                &[make_acct_mut(AccountInfo {
                    balance: U256::from(1u64),
                    nonce: 1,
                    code_hash: *EMPTY_KECCACK_HASH,
                })],
            )
            .unwrap();
        backend.update_storage(addr_a, &[(slot, value)]).unwrap();
        let output = backend.commit().unwrap();
        let mpt_root = output.root;
        store
            .write_node_updates_direct(output.node_updates)
            .unwrap();

        // Build TransitionBackend.
        let code_reader = store.make_code_reader();
        let base = {
            let state_trie = store.open_state_trie(mpt_root).unwrap();
            let provider = Arc::new(crate::mpt_wiring::StoreTrieProvider {
                store: store.clone(),
                parent_state_root: mpt_root,
            }) as Arc<dyn ethrex_trie::TrieProvider>;
            MptBackend::new_with_db(
                state_trie,
                Arc::new(NativeCrypto),
                provider,
                code_reader.clone(),
            )
        };
        let mut tb = TransitionBackend::new(base, BinaryBackend::new(), 1, mpt_root, code_reader);

        // Sanity: base storage is visible before clear_storage.
        assert_eq!(
            tb.storage(addr_a, slot).unwrap(),
            value,
            "storage must be readable from MPT base before clear_storage"
        );

        // Call clear_storage WITHOUT an account removal (the bug case).
        tb.clear_storage(addr_a).unwrap();

        // After clear_storage, the tombstone must hide base storage.
        assert_eq!(
            tb.storage(addr_a, slot).unwrap(),
            H256::zero(),
            "storage must return zero after clear_storage (tombstone hides MPT base)"
        );
    }

    // -------------------------------------------------------------------------
    // Task 6.12: restart — exercises Store::from_backend Transition path.
    //
    // 1. Create an MPT store, populate state, persist transition metadata.
    // 2. Drop the first Store handle.
    // 3. Open a SECOND Store via Store::from_backend with BackendKind::Transition
    //    — exercises the byte-2 branch that reads all three meta keys.
    // 4. Assert that the reconstructed store's transition_metadata matches.
    // 5. Verify partial-presence returns Err (load_transition_metadata semantics).
    // -------------------------------------------------------------------------
    #[test]
    fn test_transition_restart_reconstruction() {
        use crate::api::StorageBackend;
        use crate::backend::in_memory::InMemoryBackend;
        use crate::store::IN_MEMORY_COMMIT_THRESHOLD;

        let addr = make_addr(0xC3);

        // --- Step 1: build MPT state and persist transition metadata ---
        // Share an InMemoryBackend Arc across two Store instances so the second
        // Store::from_backend call (with BackendKind::Transition) sees the metadata
        // written by the first.
        let backend_arc: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());

        let store1 = Store::from_backend(
            Arc::clone(&backend_arc),
            std::path::PathBuf::from("."),
            IN_MEMORY_COMMIT_THRESHOLD,
            BackendKind::Mpt,
        )
        .unwrap();

        let mut writer = store1.new_state_writer().unwrap();
        writer
            .update_accounts(
                &[addr],
                &[make_acct_mut(AccountInfo {
                    balance: U256::from(300u64),
                    nonce: 3,
                    code_hash: *EMPTY_KECCACK_HASH,
                })],
            )
            .unwrap();
        let output = writer.commit().unwrap();
        let mpt_root = output.root;
        store1
            .write_node_updates_direct(output.node_updates)
            .unwrap();

        // Persist transition metadata (overwrites format byte to 2).
        let binary_root = H256::zero();
        store1
            .persist_transition_metadata(100, mpt_root, binary_root)
            .unwrap();

        drop(store1);

        // --- Step 2: open a SECOND Store from the SAME backend as Transition ---
        // This exercises Store::from_backend with BackendKind::Transition (byte 2)
        // and populates store2.transition_metadata from the three stored keys.
        let store2 = Store::from_backend(
            Arc::clone(&backend_arc),
            std::path::PathBuf::from("."),
            IN_MEMORY_COMMIT_THRESHOLD,
            BackendKind::Transition,
        )
        .unwrap();

        // Assert transition_metadata was populated by from_backend.
        let meta = store2
            .transition_metadata()
            .expect("transition_metadata must be set");
        assert_eq!(meta.0, 100, "switch_block must round-trip");
        assert_eq!(meta.1, mpt_root, "mpt_root must round-trip");
        assert_eq!(meta.2, binary_root, "binary_root must round-trip");

        // Reads via the TransitionBackend constructed from persisted metadata.
        let tb = store2.new_transition_state_reader(meta.0, meta.1).unwrap();
        let info = tb.account(addr).unwrap().unwrap();
        assert_eq!(
            info.nonce, 3,
            "nonce must be readable from MPT base after restart"
        );
        assert_eq!(info.balance, U256::from(300u64));

        // --- Step 3: verify load_transition_metadata partial-presence returns Err ---
        // Write only TRANSITION_SWITCH_BLOCK_KEY (missing the other two).
        {
            let partial_backend: Arc<dyn StorageBackend> =
                Arc::new(InMemoryBackend::open().unwrap());
            let partial_store = Store::from_backend(
                Arc::clone(&partial_backend),
                std::path::PathBuf::from("."),
                IN_MEMORY_COMMIT_THRESHOLD,
                BackendKind::Mpt,
            )
            .unwrap();
            // Manually write only one of the three transition keys.
            {
                let mut tx = partial_store.backend.begin_write().unwrap();
                tx.put(
                    MISC_VALUES,
                    TRANSITION_SWITCH_BLOCK_KEY,
                    &100u64.to_be_bytes(),
                )
                .unwrap();
                tx.commit().unwrap();
            }
            let result = partial_store.load_transition_metadata();
            assert!(
                result.is_err(),
                "partial key presence must return Err, got {:?}",
                result
            );
        }
    }

    // -------------------------------------------------------------------------
    // MAJOR 5 regression: stem_is_tombstoned checks persisted tombstones.
    //
    // Before the fix, stem_is_tombstoned only checked in-memory deleted_stems.
    // After a process restart (new BinaryBackend with empty deleted_stems),
    // persisted tombstones were invisible, allowing selfdestructed accounts to
    // resurrect from MPT base.
    //
    // This test:
    // 1. Opens a BinaryBackend with a StoreBinaryTrieProvider, selfdestructs an
    //    account to add a tombstone, commits (writes tombstone to disk).
    // 2. Opens a FRESH BinaryBackend pointing at the same DB.
    // 3. Asserts stem_is_tombstoned returns true (provider lookup succeeds).
    // -------------------------------------------------------------------------
    #[test]
    fn test_stem_is_tombstoned_checks_persisted_tombstones() {
        use crate::api::StorageBackend;
        use crate::backend::in_memory::InMemoryBackend;
        use crate::binary_wiring::StoreBinaryTrieProvider;
        use crate::store::IN_MEMORY_COMMIT_THRESHOLD;
        use ethrex_state_backend::NodeUpdates;

        let addr = make_addr(0xE1);
        let stem = get_stem_for_base(&addr);

        // Shared backend so we can simulate a "restart" (same disk, new backend handle).
        let backend_arc: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());

        // --- Round 1: write account, selfdestruct, commit tombstone to disk ---
        {
            let provider = Arc::new(StoreBinaryTrieProvider {
                store: Store::from_backend(
                    Arc::clone(&backend_arc),
                    std::path::PathBuf::from("."),
                    IN_MEMORY_COMMIT_THRESHOLD,
                    BackendKind::Mpt,
                )
                .unwrap(),
            }) as Arc<dyn ethrex_binary_trie::BinaryTrieProvider>;

            let mut backend = BinaryBackend::new_with_db(provider, Arc::new(|_| Ok(None)));

            // Insert an account then self-destruct it to add the stem to deleted_stems.
            backend
                .update_accounts(
                    &[addr],
                    &[make_acct_mut(AccountInfo {
                        balance: U256::from(1u64),
                        nonce: 1,
                        code_hash: *EMPTY_KECCACK_HASH,
                    })],
                )
                .unwrap();
            backend
                .update_accounts(
                    &[addr],
                    &[AccountMut {
                        account: None,
                        code: None,
                    }],
                )
                .unwrap();

            // In-memory tombstone must be visible.
            assert!(
                backend.stem_is_tombstoned(&stem).unwrap(),
                "tombstone must be visible in in-memory deleted_stems before commit"
            );

            // Commit — drains deleted_stems into NodeUpdates and writes to disk.
            let output = backend.commit().unwrap();
            match output.node_updates {
                NodeUpdates::Binary { deleted_stems, .. } => {
                    assert_eq!(
                        deleted_stems.len(),
                        1,
                        "one deleted stem expected in commit"
                    );
                    // Write the tombstone to disk via binary_commit.
                    crate::binary_wiring::binary_commit_nodes_to_disk(
                        backend_arc.as_ref(),
                        vec![],
                        deleted_stems,
                        vec![],
                    )
                    .unwrap();
                }
                _ => panic!("expected NodeUpdates::Binary"),
            }
        }

        // --- Round 2: fresh BinaryBackend, deleted_stems is empty ---
        {
            let provider2 = Arc::new(StoreBinaryTrieProvider {
                store: Store::from_backend(
                    Arc::clone(&backend_arc),
                    std::path::PathBuf::from("."),
                    IN_MEMORY_COMMIT_THRESHOLD,
                    BackendKind::Mpt,
                )
                .unwrap(),
            }) as Arc<dyn ethrex_binary_trie::BinaryTrieProvider>;

            let backend2 = BinaryBackend::new_with_db(provider2, Arc::new(|_| Ok(None)));

            // deleted_stems is empty on the fresh backend.
            // The fix: stem_is_tombstoned must also consult the provider.
            assert!(
                backend2.stem_is_tombstoned(&stem).unwrap(),
                "persisted tombstone must be visible via provider after simulated restart"
            );
        }
    }

    // -------------------------------------------------------------------------
    // Task 6.4 regression: overlay opened at binary_root, not empty on restart
    //
    // This test would have caught the Task 6.4 violation: previously,
    // new_transition_state_reader always opened an empty overlay and discarded
    // binary_root. After a restart with a non-zero binary_root, reads for
    // accounts that were written to the overlay in a prior session would
    // incorrectly fall through to the MPT base instead of returning the
    // overlay value.
    //
    // Steps:
    // 1. Build MPT state with account addr, persist transition metadata
    //    (binary_root = H256::zero() for fresh activation).
    // 2. Open second Store (Transition). Via new_transition_state_reader,
    //    write a new balance for addr (triggers CoW), commit, capture
    //    binary_root. Write binary node updates to disk. Persist updated
    //    TRANSITION_BINARY_ROOT_KEY.
    // 3. Drop second Store. Open third Store (Transition) with updated
    //    binary_root. Assert the account reads back the value from step 2,
    //    not the original MPT value. This proves the overlay was opened from
    //    disk, not empty.
    // -------------------------------------------------------------------------
    #[test]
    fn test_transition_restart_with_overlay() {
        use crate::api::StorageBackend;
        use crate::backend::in_memory::InMemoryBackend;
        use crate::binary_wiring::binary_commit_nodes_to_disk;
        use crate::store::IN_MEMORY_COMMIT_THRESHOLD;
        use ethrex_state_backend::NodeUpdates;

        let addr = make_addr(0xD4);

        // Share a single InMemoryBackend so all three Store instances see the
        // same DB state (simulating a persistent store across restarts).
        let backend_arc: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());

        // ----------------------------------------------------------------
        // Step 1: build MPT state with addr, persist transition metadata
        // (binary_root = H256::zero() for fresh activation).
        // ----------------------------------------------------------------
        let mpt_root;
        {
            let store1 = Store::from_backend(
                Arc::clone(&backend_arc),
                std::path::PathBuf::from("."),
                IN_MEMORY_COMMIT_THRESHOLD,
                BackendKind::Mpt,
            )
            .unwrap();

            let mut writer = store1.new_state_writer().unwrap();
            writer
                .update_accounts(
                    &[addr],
                    &[make_acct_mut(AccountInfo {
                        balance: U256::from(100u64),
                        nonce: 1,
                        code_hash: *EMPTY_KECCACK_HASH,
                    })],
                )
                .unwrap();
            let output = writer.commit().unwrap();
            mpt_root = output.root;
            store1
                .write_node_updates_direct(output.node_updates)
                .unwrap();

            store1
                .persist_transition_metadata(50, mpt_root, H256::zero())
                .unwrap();
        }

        // ----------------------------------------------------------------
        // Step 2: open second Store as Transition, update addr's balance
        // via the overlay (triggers CoW), commit, write binary nodes to
        // disk, persist updated TRANSITION_BINARY_ROOT_KEY.
        // ----------------------------------------------------------------
        let new_binary_root;
        {
            let store2 = Store::from_backend(
                Arc::clone(&backend_arc),
                std::path::PathBuf::from("."),
                IN_MEMORY_COMMIT_THRESHOLD,
                BackendKind::Transition,
            )
            .unwrap();

            let meta = store2
                .transition_metadata()
                .expect("transition_metadata must be set after from_backend");
            assert_eq!(
                meta.2,
                H256::zero(),
                "binary_root must be zero (fresh activation)"
            );

            // Build a TransitionBackend and update addr's balance.
            let tb_backend = store2.new_transition_state_reader(meta.0, meta.1).unwrap();

            // Unwrap StateBackend::Transition to get the inner TransitionBackend
            // so we can call StateCommitter::commit() directly.
            let mut tb = match tb_backend {
                StateBackend::Transition(inner) => *inner,
                _ => panic!("expected StateBackend::Transition"),
            };

            tb.update_accounts(
                &[addr],
                &[make_acct_mut(AccountInfo {
                    balance: U256::from(999u64),
                    nonce: 1,
                    code_hash: *EMPTY_KECCACK_HASH,
                })],
            )
            .unwrap();

            let output = tb.commit().unwrap();
            new_binary_root = output.root;

            // Write binary node diffs to disk.
            match output.node_updates {
                NodeUpdates::Binary {
                    node_diffs,
                    deleted_stems,
                    fkv_entries,
                } => {
                    binary_commit_nodes_to_disk(
                        backend_arc.as_ref(),
                        node_diffs,
                        deleted_stems,
                        fkv_entries,
                    )
                    .unwrap();
                }
                _ => panic!("expected NodeUpdates::Binary from TransitionBackend commit"),
            }

            // Persist the updated binary_root so step 3 can open the overlay.
            store2
                .persist_transition_metadata(meta.0, meta.1, new_binary_root)
                .unwrap();
        }

        assert_ne!(
            new_binary_root,
            H256::zero(),
            "new binary_root must be non-zero after overlay commit"
        );

        // ----------------------------------------------------------------
        // Step 3: open third Store as Transition with updated binary_root.
        // Assert the account reads the value from step 2 (overlay wins),
        // not the original MPT value (100). This proves the overlay was
        // opened from disk, not constructed empty.
        // ----------------------------------------------------------------
        {
            let store3 = Store::from_backend(
                Arc::clone(&backend_arc),
                std::path::PathBuf::from("."),
                IN_MEMORY_COMMIT_THRESHOLD,
                BackendKind::Transition,
            )
            .unwrap();

            let meta3 = store3
                .transition_metadata()
                .expect("transition_metadata must be set on third Store");
            assert_eq!(
                meta3.2, new_binary_root,
                "binary_root must round-trip through persist_transition_metadata"
            );

            let reader = store3
                .new_transition_state_reader(meta3.0, meta3.1)
                .unwrap();

            let info = reader
                .account(addr)
                .unwrap()
                .expect("account must be readable after overlay restart");

            assert_eq!(
                info.balance,
                U256::from(999u64),
                "balance must come from the persisted overlay (step 2 write), not MPT (100)"
            );
            assert_eq!(info.nonce, 1, "nonce must be preserved through overlay CoW");
        }
    }

    // -------------------------------------------------------------------------
    // Phase 7 regression tests (binary_transition_restart_cycle and
    // binary_transition_locked_without_flag are here because they require the
    // pub(crate) Store::from_backend API and direct backend sharing, which is
    // only accessible within ethrex-storage).
    // -------------------------------------------------------------------------

    /// Plan §6 Task 7.6 — `binary_transition_restart_cycle`.
    ///
    /// After a first `Store` activates (writes format byte 2 + transition
    /// metadata), a second `Store` opened against the **same physical backend**
    /// (shared `Arc<InMemoryBackend>`) must reconstruct as `Transition` from the
    /// format byte alone (no extra hint), and reads via that store must respect
    /// overlay→base ordering: a value written to the binary overlay shadows the
    /// MPT base; an account that exists only in the MPT base is still accessible.
    #[test]
    fn binary_transition_restart_cycle() {
        use crate::api::StorageBackend;
        use crate::backend::in_memory::InMemoryBackend;
        use crate::store::IN_MEMORY_COMMIT_THRESHOLD;

        let addr_base = make_addr(0xE0); // lives only in MPT base
        let addr_overlay = make_addr(0xE1); // written to binary overlay after activation

        // Share a single InMemoryBackend so the second Store::from_backend sees
        // the metadata written by the first (simulates a persistent restart).
        let backend_arc: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());

        // ----------------------------------------------------------------
        // Step 1: MPT store — seed addr_base, then activate (write byte 2).
        // ----------------------------------------------------------------
        let mpt_root;
        {
            let store1 = Store::from_backend(
                Arc::clone(&backend_arc),
                std::path::PathBuf::from("."),
                IN_MEMORY_COMMIT_THRESHOLD,
                BackendKind::Mpt,
            )
            .unwrap();

            let mut writer = store1.new_state_writer().unwrap();
            writer
                .update_accounts(
                    &[addr_base],
                    &[make_acct_mut(AccountInfo {
                        balance: U256::from(42u64),
                        nonce: 7,
                        code_hash: *EMPTY_KECCACK_HASH,
                    })],
                )
                .unwrap();
            let out = writer.commit().unwrap();
            mpt_root = out.root;
            store1.write_node_updates_direct(out.node_updates).unwrap();

            // Simulate activation: write format byte 2 + transition metadata.
            store1
                .persist_transition_metadata(200, mpt_root, H256::zero())
                .unwrap();
        }

        // ----------------------------------------------------------------
        // Step 2: reopen the SAME backend as Transition (no extra hint
        // beyond the backend_kind argument — the format byte on disk is 2).
        // Assert backend_kind == Transition.
        // ----------------------------------------------------------------
        let store2 = Store::from_backend(
            Arc::clone(&backend_arc),
            std::path::PathBuf::from("."),
            IN_MEMORY_COMMIT_THRESHOLD,
            BackendKind::Transition,
        )
        .unwrap();

        // backend_kind accessor reports Transition.
        assert_eq!(
            store2.backend_kind(),
            BackendKind::Transition,
            "reopened store must report BackendKind::Transition"
        );
        // transition_metadata was loaded from disk by from_backend.
        let meta = store2
            .transition_metadata()
            .expect("transition_metadata must be set after reopen");
        assert_eq!(meta.0, 200, "switch_block must round-trip");
        assert_eq!(meta.1, mpt_root, "frozen_mpt_root must round-trip");

        // ----------------------------------------------------------------
        // Step 3: reads behave overlay→base.
        //
        // addr_base exists only in the MPT base → must be readable.
        // addr_overlay is written to the binary overlay → overlay value wins.
        // ----------------------------------------------------------------
        let mut tb = match store2.new_transition_state_reader(meta.0, meta.1).unwrap() {
            StateBackend::Transition(inner) => *inner,
            _ => panic!("expected StateBackend::Transition"),
        };

        // Base-only account is visible through the overlay→base fallthrough.
        let base_info = tb
            .account(addr_base)
            .unwrap()
            .expect("addr_base must be readable from MPT base");
        assert_eq!(base_info.nonce, 7, "nonce from MPT base");
        assert_eq!(
            base_info.balance,
            U256::from(42u64),
            "balance from MPT base"
        );

        // Overlay write shadows the base.
        tb.update_accounts(
            &[addr_overlay],
            &[make_acct_mut(AccountInfo {
                balance: U256::from(999u64),
                nonce: 1,
                code_hash: *EMPTY_KECCACK_HASH,
            })],
        )
        .unwrap();

        let overlay_info = tb
            .account(addr_overlay)
            .unwrap()
            .expect("addr_overlay must be readable from overlay");
        assert_eq!(
            overlay_info.balance,
            U256::from(999u64),
            "overlay write must shadow base"
        );
    }

    // -------------------------------------------------------------------------
    // EIP-aware boundary tests: scenarios where MPT base + binary overlay
    // semantics interact in non-obvious ways.
    // -------------------------------------------------------------------------

    /// Helper: construct a fully wired TransitionBackend over a store seeded
    /// with `mpt_root`. Mirrors the wiring that other tests duplicate inline.
    fn build_transition_backend(store: &Store, mpt_root: H256) -> TransitionBackend {
        let code_reader = store.make_code_reader();
        let base = {
            let state_trie = store.open_state_trie(mpt_root).unwrap();
            let provider = Arc::new(crate::mpt_wiring::StoreTrieProvider {
                store: store.clone(),
                parent_state_root: mpt_root,
            }) as Arc<dyn ethrex_trie::TrieProvider>;
            MptBackend::new_with_db(
                state_trie,
                Arc::new(NativeCrypto),
                provider,
                code_reader.clone(),
            )
        };
        let overlay = BinaryBackend::new_with_db(
            Arc::new(crate::binary_wiring::StoreBinaryTrieProvider {
                store: store.clone(),
            }) as Arc<dyn ethrex_binary_trie::BinaryTrieProvider>,
            code_reader.clone(),
        );
        TransitionBackend::new(base, overlay, 1, mpt_root, code_reader)
    }

    /// Helper: seed an MPT account with optional storage slots; returns the
    /// resulting state root.
    fn seed_mpt(
        store: &Store,
        accounts: &[(Address, AccountInfo)],
        storage: &[(Address, &[(H256, H256)])],
    ) -> H256 {
        let mut backend = store.new_state_writer().unwrap();
        let addrs: Vec<Address> = accounts.iter().map(|(a, _)| *a).collect();
        let muts: Vec<AccountMut> = accounts
            .iter()
            .map(|(_, info)| make_acct_mut(*info))
            .collect();
        backend.update_accounts(&addrs, &muts).unwrap();
        for (addr, slots) in storage {
            backend.update_storage(*addr, slots).unwrap();
        }
        let output = backend.commit().unwrap();
        store
            .write_node_updates_direct(output.node_updates)
            .unwrap();
        output.root
    }

    // -------------------------------------------------------------------------
    // Test 1 (EIP-2935 BLOCKHASH history contract): pre-switch storage slot
    // gets a non-zero overlay overwrite. Must read the overlay value, not the
    // pre-switch MPT value. This is the COMMON case: history contract slots
    // (block_number % 8192) get rewritten with new block hashes every 8192
    // blocks.
    // -------------------------------------------------------------------------
    #[test]
    fn test_history_contract_storage_slot_reuse_across_switch() {
        let history = make_addr(0xE1);
        let slot = H256::from_low_u64_be(123);
        let v_old = H256::from_low_u64_be(0x1111);
        let v_new = H256::from_low_u64_be(0x2222);

        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();
        let mpt_root = seed_mpt(
            &store,
            &[(
                history,
                AccountInfo {
                    nonce: 1,
                    balance: U256::from(0u64),
                    code_hash: *EMPTY_KECCACK_HASH,
                },
            )],
            &[(history, &[(slot, v_old)])],
        );

        let mut tb = build_transition_backend(&store, mpt_root);

        // Sanity: pre-switch read returns V_old from MPT base.
        assert_eq!(
            tb.storage(history, slot).unwrap(),
            v_old,
            "pre-switch storage must come from MPT base"
        );

        // Post-switch overwrite (CoW account first, then storage).
        tb.update_accounts(
            &[history],
            &[make_acct_mut(AccountInfo {
                nonce: 1,
                balance: U256::from(0u64),
                code_hash: *EMPTY_KECCACK_HASH,
            })],
        )
        .unwrap();
        tb.update_storage(history, &[(slot, v_new)]).unwrap();

        assert_eq!(
            tb.storage(history, slot).unwrap(),
            v_new,
            "after overlay write, read must return overlay value, not MPT"
        );
    }

    // -------------------------------------------------------------------------
    // Test 2 (cross-boundary correctness hazard): post-switch zero-write to a
    // slot that holds a non-zero pre-switch value MUST hide the pre-switch
    // value. The binary trie deletes leaves on zero-write; `TransitionBackend`
    // must distinguish "explicitly zeroed in overlay" from "absent in overlay,
    // fall through to base".
    //
    // If this test fails, there is a real correctness bug in the storage()
    // read path: post-switch SSTORE 0 would not actually clear the slot.
    // -------------------------------------------------------------------------
    #[test]
    fn test_storage_zero_write_hides_pre_switch_value() {
        let addr = make_addr(0xE2);
        let slot = H256::from_low_u64_be(42);
        let v_old = H256::from_low_u64_be(0xAAAA);

        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();
        let mpt_root = seed_mpt(
            &store,
            &[(
                addr,
                AccountInfo {
                    nonce: 1,
                    balance: U256::from(0u64),
                    code_hash: *EMPTY_KECCACK_HASH,
                },
            )],
            &[(addr, &[(slot, v_old)])],
        );

        let mut tb = build_transition_backend(&store, mpt_root);

        // Pre-switch read returns the non-zero MPT value.
        assert_eq!(tb.storage(addr, slot).unwrap(), v_old);

        // Post-switch zero-write (CoW account, then SSTORE(slot, 0)).
        tb.update_accounts(
            &[addr],
            &[make_acct_mut(AccountInfo {
                nonce: 1,
                balance: U256::from(0u64),
                code_hash: *EMPTY_KECCACK_HASH,
            })],
        )
        .unwrap();
        tb.update_storage(addr, &[(slot, H256::zero())]).unwrap();

        assert_eq!(
            tb.storage(addr, slot).unwrap(),
            H256::zero(),
            "post-switch zero-write must hide pre-switch MPT value (no fall-through)"
        );
    }

    // -------------------------------------------------------------------------
    // Test 3: account CoW does not migrate storage slots. After CoW, untouched
    // slots still resolve via fall-through to base; explicitly-overlay-written
    // slots resolve via overlay.
    // -------------------------------------------------------------------------
    #[test]
    fn test_storage_read_falls_through_after_account_cow() {
        let addr = make_addr(0xE3);
        let s1 = H256::from_low_u64_be(1);
        let s2 = H256::from_low_u64_be(2);
        let v1 = H256::from_low_u64_be(0xBEEF);
        let v2 = H256::from_low_u64_be(0xCAFE);
        let v1_new = H256::from_low_u64_be(0xDEAD);

        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();
        let mpt_root = seed_mpt(
            &store,
            &[(
                addr,
                AccountInfo {
                    nonce: 1,
                    balance: U256::from(10u64),
                    code_hash: *EMPTY_KECCACK_HASH,
                },
            )],
            &[(addr, &[(s1, v1), (s2, v2)])],
        );

        let mut tb = build_transition_backend(&store, mpt_root);

        // Trigger account-level CoW (balance change) without touching storage.
        tb.update_accounts(
            &[addr],
            &[make_acct_mut(AccountInfo {
                nonce: 1,
                balance: U256::from(99u64),
                code_hash: *EMPTY_KECCACK_HASH,
            })],
        )
        .unwrap();

        // Both slots still readable from base (account CoW does NOT pull storage).
        assert_eq!(tb.storage(addr, s1).unwrap(), v1, "s1 must fall through");
        assert_eq!(tb.storage(addr, s2).unwrap(), v2, "s2 must fall through");

        // Overlay-write s1, leaving s2 untouched.
        tb.update_storage(addr, &[(s1, v1_new)]).unwrap();

        assert_eq!(
            tb.storage(addr, s1).unwrap(),
            v1_new,
            "s1 must read from overlay after explicit write"
        );
        assert_eq!(
            tb.storage(addr, s2).unwrap(),
            v2,
            "s2 must still fall through to base (untouched)"
        );
    }

    // -------------------------------------------------------------------------
    // Test 4 (EIP-6780 SELFDESTRUCT scope-limit): post-Cancun, SELFDESTRUCT
    // on a pre-switch contract that wasn't created in the same tx only
    // transfers balance — does NOT delete. The EVM signals this by passing
    // `Some(info)` to update_accounts (a balance update), not `None`. The
    // backend must NOT write a tombstone in that case.
    // -------------------------------------------------------------------------
    #[test]
    fn test_selfdestruct_pre_switch_contract_post_cancun_no_tombstone() {
        let addr = make_addr(0xE4);
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();

        // Pre-seed an MPT contract (nonce=2, balance=1000, EMPTY code for simplicity).
        let mpt_root = seed_mpt(
            &store,
            &[(
                addr,
                AccountInfo {
                    nonce: 2,
                    balance: U256::from(1000u64),
                    code_hash: *EMPTY_KECCACK_HASH,
                },
            )],
            &[],
        );

        let mut tb = build_transition_backend(&store, mpt_root);
        let stem = get_stem_for_base(&addr);

        // EIP-6780 path: post-Cancun selfdestruct of a non-same-tx-created
        // contract is a balance transfer to zero, NOT a removal. Pass
        // Some(info_with_zero_balance), not None.
        tb.update_accounts(
            &[addr],
            &[make_acct_mut(AccountInfo {
                nonce: 2,
                balance: U256::zero(),
                code_hash: *EMPTY_KECCACK_HASH,
            })],
        )
        .unwrap();

        // The account must still be readable (no tombstone).
        let info = tb.account(addr).unwrap();
        assert!(
            info.is_some(),
            "account must remain readable; no tombstone for EIP-6780 balance transfer"
        );
        let info = info.unwrap();
        assert_eq!(info.balance, U256::zero(), "balance must reflect transfer");
        assert_eq!(info.nonce, 2, "nonce must be preserved");

        // The stem must NOT be tombstoned.
        assert!(
            !tb.overlay.stem_is_tombstoned(&stem).unwrap(),
            "stem must NOT be tombstoned for EIP-6780 balance-only update"
        );
    }

    // -------------------------------------------------------------------------
    // Test 5 (EIP-7702 set-EOA-code on pre-switch EOA): a pre-switch EOA
    // gets a delegation pointer post-switch. CoW pulls the EOA, then the
    // update overwrites code_hash and dual-writes the bytecode to AccountCodes.
    // -------------------------------------------------------------------------
    #[test]
    fn test_eip7702_delegation_on_pre_switch_eoa() {
        let addr = make_addr(0xE5);
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();

        // Pre-seed an MPT EOA (no code).
        let mpt_root = seed_mpt(
            &store,
            &[(
                addr,
                AccountInfo {
                    nonce: 10,
                    balance: U256::from(100u64),
                    code_hash: *EMPTY_KECCACK_HASH,
                },
            )],
            &[],
        );

        let mut tb = build_transition_backend(&store, mpt_root);

        // Build the EIP-7702 delegation pointer: 0xef0100 || target_addr (23 bytes).
        let target = make_addr(0xAB);
        let mut delegation = vec![0xefu8, 0x01, 0x00];
        delegation.extend_from_slice(target.as_bytes());
        assert_eq!(delegation.len(), 23, "EIP-7702 delegation is 23 bytes");

        let delegation_code =
            Code::from_bytecode(bytes::Bytes::copy_from_slice(&delegation), &NativeCrypto);
        let delegation_hash = delegation_code.hash;

        // Apply the SET_CODE: nonce bumps by 1 (per spec), balance unchanged,
        // code_hash = delegation_hash, code = delegation bytecode.
        let mut acct_mut = make_acct_mut(AccountInfo {
            nonce: 11,
            balance: U256::from(100u64),
            code_hash: delegation_hash,
        });
        acct_mut.code = Some(ethrex_state_backend::CodeMut {
            code: Some(delegation.clone()),
        });

        tb.update_accounts(&[addr], &[acct_mut]).unwrap();

        // Same-session read must reflect the overlay write: code_hash and
        // nonce are now the delegation's, not the pre-switch EOA's.
        let info = tb
            .account(addr)
            .unwrap()
            .expect("account readable after delegation");
        assert_eq!(
            info.code_hash, delegation_hash,
            "overlay must reflect new code_hash from delegation"
        );
        assert_eq!(info.nonce, 11, "nonce bump must reflect in overlay");

        // The MerkleOutput's code_updates carries the bytecode to be
        // dual-written to AccountCodes (so subsequent code() reads via
        // code_reader can find it post-restart).
        let output = tb.commit().unwrap();
        assert!(
            output
                .code_updates
                .iter()
                .any(|(h, _)| *h == delegation_hash),
            "delegation bytecode must be in code_updates for AccountCodes dual-write"
        );
    }

    // -------------------------------------------------------------------------
    // Test 6 (EIP-161 empty account deletion): a pre-switch account with
    // (nonce=0, balance=0, code=EMPTY) that gets touched post-switch should be
    // deleted via a tombstone. The EVM signals this with `update_accounts(None)`.
    // -------------------------------------------------------------------------
    #[test]
    fn test_empty_account_deletion_writes_tombstone() {
        let addr = make_addr(0xE6);
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();

        // Pre-seed an empty account in MPT (per EIP-161 definition: nonce=0,
        // balance=0, code=EMPTY).
        let mpt_root = seed_mpt(
            &store,
            &[(
                addr,
                AccountInfo {
                    nonce: 0,
                    balance: U256::zero(),
                    code_hash: *EMPTY_KECCACK_HASH,
                },
            )],
            &[],
        );

        let mut tb = build_transition_backend(&store, mpt_root);

        // Sanity: pre-deletion read finds the empty account.
        assert!(
            tb.account(addr).unwrap().is_some(),
            "empty MPT account must be readable before deletion"
        );

        // Post-touch deletion (empty-account cleanup).
        tb.update_accounts(
            &[addr],
            &[AccountMut {
                account: None,
                code: None,
            }],
        )
        .unwrap();

        // Tombstone must be written; subsequent read must return None.
        let stem = get_stem_for_base(&addr);
        assert!(
            tb.overlay.stem_is_tombstoned(&stem).unwrap(),
            "tombstone must be written for empty account deletion"
        );
        assert!(
            tb.account(addr).unwrap().is_none(),
            "deleted account must read as None (no MPT fallthrough)"
        );
    }

    // -------------------------------------------------------------------------
    // Test 7: CREATE2 collision with a pre-switch contract. The EVM-side
    // collision check just calls account(addr); a Some result means the address
    // is occupied and CREATE2 must fail. The transition backend must surface
    // pre-switch contracts here so the collision check works.
    // -------------------------------------------------------------------------
    #[test]
    fn test_create2_collision_with_pre_switch_contract() {
        let addr = make_addr(0xE7);
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();

        // Seed AccountCodes with non-empty bytecode and reference it from MPT.
        let bytecode: Vec<u8> = vec![0x60, 0x80, 0x60, 0x40, 0x52]; // PUSH1 0x80 PUSH1 0x40 MSTORE
        let code = Code::from_bytecode(bytes::Bytes::copy_from_slice(&bytecode), &NativeCrypto);
        let code_hash = code.hash;
        store
            .write(ACCOUNT_CODES, code_hash.0.to_vec(), encode_code(&code))
            .unwrap();
        store
            .write(
                ACCOUNT_CODE_METADATA,
                code_hash.0.to_vec(),
                (bytecode.len() as u64).to_be_bytes().to_vec(),
            )
            .unwrap();

        let mpt_root = seed_mpt(
            &store,
            &[(
                addr,
                AccountInfo {
                    nonce: 1,
                    balance: U256::zero(),
                    code_hash,
                },
            )],
            &[],
        );

        let tb = build_transition_backend(&store, mpt_root);

        // CREATE2 collision check: account() must return Some for the existing
        // pre-switch contract. The EVM uses this to decide whether to revert.
        let info = tb
            .account(addr)
            .unwrap()
            .expect("pre-switch contract must be visible to collision check");
        assert_eq!(
            info.code_hash, code_hash,
            "code_hash must come from MPT base (proves contract presence)"
        );
        assert_eq!(info.nonce, 1, "nonce must come from MPT base");
    }

    // -------------------------------------------------------------------------
    // Test 8: Intra-block read-after-write across multiple updates. The EVM
    // requires that tx2 in the same block sees tx1's state changes. The
    // TransitionBackend must reflect uncommitted overlay writes immediately
    // through subsequent reads — never returning the stale MPT base value
    // after a CoW write has been applied.
    // -------------------------------------------------------------------------
    #[test]
    fn test_intra_block_read_after_write_returns_overlay_value() {
        let addr = make_addr(0xE8);
        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();
        let mpt_root = seed_mpt(
            &store,
            &[(
                addr,
                AccountInfo {
                    nonce: 1,
                    balance: U256::from(10u64),
                    code_hash: *EMPTY_KECCACK_HASH,
                },
            )],
            &[],
        );

        let mut tb = build_transition_backend(&store, mpt_root);

        // tx1: balance 10 → 20.
        tb.update_accounts(
            &[addr],
            &[make_acct_mut(AccountInfo {
                nonce: 1,
                balance: U256::from(20u64),
                code_hash: *EMPTY_KECCACK_HASH,
            })],
        )
        .unwrap();

        // tx2 read: must see 20, not 10.
        assert_eq!(
            tb.account(addr).unwrap().unwrap().balance,
            U256::from(20u64),
            "intra-block read must return tx1's overlay value"
        );

        // tx3: balance 20 → 30.
        tb.update_accounts(
            &[addr],
            &[make_acct_mut(AccountInfo {
                nonce: 1,
                balance: U256::from(30u64),
                code_hash: *EMPTY_KECCACK_HASH,
            })],
        )
        .unwrap();

        // tx4 read: must see 30.
        assert_eq!(
            tb.account(addr).unwrap().unwrap().balance,
            U256::from(30u64),
            "intra-block read must return latest overlay value"
        );
    }

    // -------------------------------------------------------------------------
    // Test 9: A TransitionBackend whose updates are dropped without commit
    // must not leak CoW state to disk. Drop the merkleizer/TB, open a fresh
    // backend over the same store, and verify the overlay is empty (no FKV
    // rows, no tombstones, no node updates).
    // -------------------------------------------------------------------------
    #[test]
    fn test_intra_tx_revert_does_not_leak_cow() {
        use crate::api::StorageBackend;
        use crate::backend::in_memory::InMemoryBackend;
        use crate::store::IN_MEMORY_COMMIT_THRESHOLD;

        let addr = make_addr(0xE9);

        let backend_arc: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::open().unwrap());
        let store = Store::from_backend(
            Arc::clone(&backend_arc),
            std::path::PathBuf::from("."),
            IN_MEMORY_COMMIT_THRESHOLD,
            BackendKind::Mpt,
        )
        .unwrap();

        let mpt_root = seed_mpt(
            &store,
            &[(
                addr,
                AccountInfo {
                    nonce: 1,
                    balance: U256::from(10u64),
                    code_hash: *EMPTY_KECCACK_HASH,
                },
            )],
            &[],
        );

        // Apply an update to the TransitionBackend.
        {
            let mut tb = build_transition_backend(&store, mpt_root);
            tb.update_accounts(
                &[addr],
                &[make_acct_mut(AccountInfo {
                    nonce: 1,
                    balance: U256::from(999u64),
                    code_hash: *EMPTY_KECCACK_HASH,
                })],
            )
            .unwrap();
            // Drop tb WITHOUT calling commit() or persisting node updates.
        }

        // Inspect the disk state directly: no BINARY_FLATKEYVALUE rows should
        // exist, no BINARY_TRIE_NODES tombstones (`0xFE` prefix), and no
        // META_ROOT_HASH (`0xFF, 'h'`) should have been written.
        let read_tx = backend_arc.begin_read().unwrap();

        // Scan BINARY_FLATKEYVALUE — must be empty.
        let fkv_iter = read_tx
            .prefix_iterator(crate::api::tables::BINARY_FLATKEYVALUE, &[])
            .unwrap();
        let fkv_count = fkv_iter.count();
        assert_eq!(
            fkv_count, 0,
            "BINARY_FLATKEYVALUE must be empty when TB is dropped without commit"
        );

        // Scan BINARY_TRIE_NODES — also must be empty (no nodes, no tombstones).
        let nodes_iter = read_tx
            .prefix_iterator(crate::api::tables::BINARY_TRIE_NODES, &[])
            .unwrap();
        let nodes_count = nodes_iter.count();
        assert_eq!(
            nodes_count, 0,
            "BINARY_TRIE_NODES must be empty when TB is dropped without commit"
        );
    }

    // -------------------------------------------------------------------------
    // Test 10: MptBackend::storage_root_cache must remain immutable under
    // overlay writes. After CoW writes shadow an account, untouched slots
    // continue to read from MPT base via the cache. The cache itself must
    // never be invalidated by transition writes (which only mutate the binary
    // overlay, never the frozen MPT).
    // -------------------------------------------------------------------------
    #[test]
    fn test_storage_root_cache_immutable_under_overlay_writes() {
        let addr_a = make_addr(0xEA);
        let addr_b = make_addr(0xEB);
        let s1 = H256::from_low_u64_be(1);
        let s2 = H256::from_low_u64_be(2);
        let v1 = H256::from_low_u64_be(0x100);
        let v2 = H256::from_low_u64_be(0x200);

        let store = Store::new(".", EngineType::InMemory, BackendKind::Mpt).unwrap();
        let mpt_root = seed_mpt(
            &store,
            &[
                (
                    addr_a,
                    AccountInfo {
                        nonce: 1,
                        balance: U256::from(10u64),
                        code_hash: *EMPTY_KECCACK_HASH,
                    },
                ),
                (
                    addr_b,
                    AccountInfo {
                        nonce: 1,
                        balance: U256::from(20u64),
                        code_hash: *EMPTY_KECCACK_HASH,
                    },
                ),
            ],
            &[(addr_a, &[(s1, v1)]), (addr_b, &[(s2, v2)])],
        );

        let mut tb = build_transition_backend(&store, mpt_root);

        // Prime the storage_root_cache with reads on both addresses.
        assert_eq!(tb.storage(addr_a, s1).unwrap(), v1);
        assert_eq!(tb.storage(addr_b, s2).unwrap(), v2);

        // CoW addr_a + write a new storage slot to overlay.
        tb.update_accounts(
            &[addr_a],
            &[make_acct_mut(AccountInfo {
                nonce: 1,
                balance: U256::from(99u64),
                code_hash: *EMPTY_KECCACK_HASH,
            })],
        )
        .unwrap();
        let s3 = H256::from_low_u64_be(3);
        let v3 = H256::from_low_u64_be(0x300);
        tb.update_storage(addr_a, &[(s3, v3)]).unwrap();

        // After overlay writes:
        //   addr_a's untouched slot s1 still resolves via base (MPT trie + cache).
        //   addr_b's slot s2 still resolves correctly (no cross-contamination).
        assert_eq!(
            tb.storage(addr_a, s1).unwrap(),
            v1,
            "addr_a's untouched MPT slot s1 must still read from base"
        );
        assert_eq!(
            tb.storage(addr_b, s2).unwrap(),
            v2,
            "addr_b's MPT slot s2 must remain readable (no cache poisoning)"
        );

        // Direct base inspection: account_state for addr_a in MPT base must
        // be unchanged from constructor-time (balance still 10, not 99).
        let base_acct = tb
            .base
            .account(addr_a)
            .unwrap()
            .expect("addr_a in base MPT");
        assert_eq!(
            base_acct.balance,
            U256::from(10u64),
            "MPT base must NOT reflect overlay's balance update"
        );
    }
}
