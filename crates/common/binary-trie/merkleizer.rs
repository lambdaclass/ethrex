//! Single-tree, level-parallel binary trie merkleizer.
//!
//! Replaces the earlier 16-shard worker architecture (commits `e3d8721abb` /
//! `99604ceea5`) with a design that targets the actual bottleneck: BLAKE3
//! throughput on the dirty-node frontier.
//!
//! ## Architecture
//!
//! Apply is serial (`feed_updates` calls `BinaryTrieState::apply_account_update`
//! in a simple loop; apply is ~1-5 ms for a 10k-update block and is not worth
//! parallelising). Merkelize is parallel: `finalize` calls
//! `BinaryTrieState::merkelize_parallel`, which does one serial BFS to collect
//! dirty nodes by depth, then processes levels bottom-up with `rayon::par_iter`
//! within each level. See `state.rs` for the interior-mutability design note.
//!
//! ## Sparse StemNode hashing
//!
//! EIP-7864: `hash([0]*64) = [0]*32`. A StemNode with K occupied sub-indices
//! rehashes in ~K·8 BLAKE3 calls (only the non-zero paths from leaves to the
//! subtree root are traversed) instead of ~511 for a full 256-leaf tree. This
//! is implemented in `merkle::compute_subtree_root` via `sparse_subtree`.

use std::sync::Arc;

use ethrex_common::{
    Address, H256,
    types::{AccountUpdate, Code},
};
use ethrex_state_backend::{MerkleOutput, NodeUpdates, StateError};
use rustc_hash::FxHashMap;

use crate::{BinaryTrieState, backend::BinaryTrieProvider, key_mapping::get_stem_for_base};

// ---------------------------------------------------------------------------
// Deduplication helper
// ---------------------------------------------------------------------------

/// Merge `update` into `map` with last-write-wins semantics.
///
/// If an entry for `update.address` already exists:
/// - `info` is overwritten if `update.info` is `Some`.
/// - `code` is overwritten if `update.code` is `Some`.
/// - `added_storage` is merged (newer values overwrite older ones per slot).
/// - `removed` / `removed_storage` are OR-ed (a removal always takes effect).
fn merge_update(map: &mut FxHashMap<Address, AccountUpdate>, update: AccountUpdate) {
    use std::collections::hash_map::Entry;
    match map.entry(update.address) {
        Entry::Vacant(v) => {
            v.insert(update);
        }
        Entry::Occupied(mut o) => {
            let existing = o.get_mut();
            if update.removed {
                existing.removed = true;
            }
            if update.removed_storage {
                existing.removed_storage = true;
            }
            if let Some(info) = update.info {
                existing.info = Some(info);
            }
            if let Some(code) = update.code {
                existing.code = Some(code);
            }
            for (slot, value) in update.added_storage {
                existing.added_storage.insert(slot, value);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// BinaryMerkleizer
// ---------------------------------------------------------------------------

/// Single-tree, level-parallel binary trie merkleizer.
///
/// Created via [`BinaryMerkleizer::new`] (standard path) or
/// [`BinaryMerkleizer::new_bal`] (BAL-optimised: skips last-write-wins dedup).
///
/// Call [`feed_updates`](BinaryMerkleizer::feed_updates) one or more times,
/// then [`finalize`](BinaryMerkleizer::finalize) to get the [`MerkleOutput`].
///
/// Internally: apply is a serial loop over `BinaryTrieState::apply_account_update`;
/// merkelize parallelises by tree level via `BinaryTrieState::merkelize_parallel`.
/// No worker threads, no channels, no `catch_unwind` wrappers.
pub struct BinaryMerkleizer {
    /// The single binary trie holding all state updates for this block.
    state: BinaryTrieState,
    /// Rayon pool used for level-parallel merkelize.
    pool: Arc<rayon::ThreadPool>,
    /// Provider for cold-cache reads (not used in the apply path but kept for
    /// interface parity with `MptMerkleizer` and future Phase-5 warm loading).
    #[allow(dead_code)]
    provider: Arc<dyn BinaryTrieProvider>,
    /// Code deployments observed during `feed_updates`.
    code_updates: Vec<(H256, Code)>,
    /// Per-leaf mutations accumulated during `feed_updates` for FKV emission.
    fkv_entries: Vec<([u8; 32], Option<[u8; 32]>)>,
    /// Stems tombstoned by SELFDESTRUCT / full removal.
    deleted_stems: Vec<[u8; 31]>,
    /// Accumulator for witness pre-computation (only allocated when
    /// `precompute_witnesses` is `true`).
    accumulator: Option<FxHashMap<Address, AccountUpdate>>,
    /// When `true`, `feed_updates` skips last-write-wins dedup (BAL mode: the
    /// caller guarantees updates are already deduplicated).
    bal_mode: bool,
    /// Used in the standard (non-BAL) path to detect duplicate addresses.
    pending: FxHashMap<Address, AccountUpdate>,
}

impl BinaryMerkleizer {
    /// Create a standard merkleizer rooted at the live binary head.
    ///
    /// Opens [`BinaryTrieState`] via `provider.trie_backend()`, which for
    /// production providers returns a cache-aware backend serving reads
    /// through the in-memory layer cache before disk. The merkleizer's trie
    /// is therefore rooted at the FULL post-parent state, so reads via
    /// `state.trie_get` during apply (e.g. existing `code_size` lookups) and
    /// from any read-path gate function see all accounts modified at any
    /// prior block — matching `MptMerkleizer`'s behavior of opening at
    /// `parent_state_root` and lazy-loading through the provider.
    ///
    /// For test / genesis-bootstrap providers (`EmptyBinaryTrieProvider`),
    /// the default `trie_backend()` returns [`crate::db::EmptyTrieBackend`],
    /// which yields an empty in-memory state — preserving the prior
    /// "empty trie" bootstrap path.
    ///
    /// `_parent_root` is currently informational only; the live head is
    /// resolved via the trie backend's META_ROOT lookup. The parameter is
    /// kept for symmetry with `MptMerkleizer::new` and future validation.
    ///
    /// `feed_updates` deduplicates by address (last-write-wins) and applies
    /// each update to `BinaryTrieState` immediately.
    pub fn new(
        _parent_root: H256,
        precompute_witnesses: bool,
        provider: Arc<dyn BinaryTrieProvider>,
        pool: Arc<rayon::ThreadPool>,
    ) -> Result<Self, StateError> {
        let state = Self::open_state(provider.as_ref())?;
        Ok(Self {
            state,
            pool,
            provider,
            code_updates: Vec::new(),
            fkv_entries: Vec::new(),
            deleted_stems: Vec::new(),
            accumulator: if precompute_witnesses {
                Some(FxHashMap::default())
            } else {
                None
            },
            bal_mode: false,
            pending: FxHashMap::default(),
        })
    }

    /// Create a BAL-optimised merkleizer rooted at the live binary head.
    ///
    /// Same backing as [`Self::new`]; only the apply path differs (BAL mode
    /// expects pre-deduplicated updates and skips the merge-with-previous
    /// step).
    pub fn new_bal(
        _parent_root: H256,
        precompute_witnesses: bool,
        provider: Arc<dyn BinaryTrieProvider>,
        pool: Arc<rayon::ThreadPool>,
    ) -> Result<Self, StateError> {
        let state = Self::open_state(provider.as_ref())?;
        Ok(Self {
            state,
            pool,
            provider,
            code_updates: Vec::new(),
            fkv_entries: Vec::new(),
            deleted_stems: Vec::new(),
            accumulator: if precompute_witnesses {
                Some(FxHashMap::default())
            } else {
                None
            },
            bal_mode: true,
            pending: FxHashMap::default(),
        })
    }

    /// Open a `BinaryTrieState` via the provider, deferring backend + table
    /// choice to the provider impl.
    fn open_state(provider: &dyn BinaryTrieProvider) -> Result<BinaryTrieState, StateError> {
        provider
            .open_state()
            .map_err(|e| StateError::Other(format!("BinaryMerkleizer open: {e}")))
    }

    /// Feed a batch of account updates.
    ///
    /// In standard mode, updates for the same address are merged (last-write-wins)
    /// into `self.pending` and not yet applied to the trie. They are flushed and
    /// applied in `finalize`. This ensures correct last-write-wins semantics when
    /// the caller calls `feed_updates` multiple times.
    ///
    /// In BAL mode the caller guarantees updates are already deduplicated, so
    /// each update is applied directly to the trie.
    pub fn feed_updates(&mut self, updates: Vec<AccountUpdate>) -> Result<(), StateError> {
        if self.bal_mode {
            for update in updates {
                // Accumulate for witness pre-computation.
                if let Some(acc) = &mut self.accumulator {
                    merge_update(acc, update.clone());
                }
                self.apply_single_update(update)?;
            }
        } else {
            for update in updates {
                // Accumulate for witness pre-computation.
                if let Some(acc) = &mut self.accumulator {
                    merge_update(acc, update.clone());
                }
                merge_update(&mut self.pending, update);
            }
        }
        Ok(())
    }

    /// Finalize merkleization.
    ///
    /// 1. Flush any pending (standard mode) updates to the trie.
    /// 2. Call `merkelize_parallel` to get the root hash.
    /// 3. Drain `node_diffs` from the `NodeStore`.
    /// 4. Return `MerkleOutput`.
    pub fn finalize(mut self) -> Result<MerkleOutput, StateError> {
        // Flush standard-mode pending updates.
        if !self.bal_mode {
            let pending = std::mem::take(&mut self.pending);
            for (_addr, update) in pending {
                self.apply_single_update(update)?;
            }
        }

        // Level-parallel merkelize.
        let root_bytes = self.state.merkelize_parallel(&self.pool);

        // Drain fkv_entries accumulated during apply.
        let (_parent, block_diffs) = self.state.take_block_diffs(root_bytes);
        self.fkv_entries.extend(block_diffs);

        // Drain dirty nodes from the NodeStore.
        // Pass root_bytes so META_ROOT_HASH is stored for reader root-pinning.
        let node_diffs = self.state.take_trie_dirty(root_bytes);

        let accumulated_updates = self
            .accumulator
            .take()
            .map(|acc| acc.into_values().collect());

        Ok(MerkleOutput {
            root: H256(root_bytes),
            node_updates: NodeUpdates::Binary {
                node_diffs,
                deleted_stems: self.deleted_stems,
                fkv_entries: self.fkv_entries,
            },
            code_updates: self.code_updates,
            accumulated_updates,
        })
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Apply a single `AccountUpdate` to the trie, collecting side effects
    /// (deleted stems, code updates) into the corresponding accumulators.
    fn apply_single_update(&mut self, update: AccountUpdate) -> Result<(), StateError> {
        // Track SELFDESTRUCT / full removal for deleted_stems.
        if update.removed {
            let stem = get_stem_for_base(&update.address);
            self.deleted_stems.push(stem);
        }

        // Collect code updates for the output.
        if let Some(ref info) = update.info
            && let Some(ref code) = update.code
        {
            self.code_updates.push((info.code_hash, code.clone()));
        }

        self.state
            .apply_account_update(&update)
            .map_err(|e| StateError::Other(format!("BinaryMerkleizer apply: {e}")))?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::{
        Address, H256, U256,
        types::{AccountInfo, AccountUpdate, Code},
    };
    use std::sync::Arc;

    fn make_pool() -> Arc<rayon::ThreadPool> {
        Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(4)
                .build()
                .expect("pool"),
        )
    }

    fn make_provider() -> Arc<dyn BinaryTrieProvider> {
        Arc::new(crate::backend::EmptyBinaryTrieProvider)
    }

    fn random_address(seed: u64) -> Address {
        let mut bytes = [0u8; 20];
        let seed_bytes = seed.to_le_bytes();
        for (i, b) in seed_bytes.iter().enumerate() {
            bytes[i % 20] ^= *b;
        }
        Address::from(bytes)
    }

    fn account_update(addr: Address, balance: u64) -> AccountUpdate {
        let mut upd = AccountUpdate::new(addr);
        upd.info = Some(AccountInfo {
            balance: U256::from(balance),
            nonce: 0,
            code_hash: *ethrex_common::constants::EMPTY_KECCACK_HASH,
        });
        upd
    }

    // -----------------------------------------------------------------------
    // Task 4.14: round-trip parity
    // -----------------------------------------------------------------------

    /// Feed N updates into `BinaryMerkleizer`, call `finalize`; separately build
    /// a `BinaryTrieState` with the same updates, call serial `state_root()`.
    /// Assert bit-for-bit equality. Covers:
    ///   - single account
    ///   - 10 accounts
    ///   - 100 accounts
    ///   - SELFDESTRUCT (removed=true)
    ///   - code deployment
    #[test]
    fn test_merkleizer_round_trip() {
        let pool = make_pool();
        let provider = make_provider();

        // --- single account ---
        {
            let addr = random_address(1);
            let updates = vec![account_update(addr, 1_000_000)];
            let mt_root = run_merkleizer(updates.clone(), Arc::clone(&pool), Arc::clone(&provider));
            let st_root = run_oracle(updates);
            assert_eq!(mt_root, st_root, "single account root mismatch");
        }

        // --- 10 accounts ---
        {
            let updates: Vec<AccountUpdate> = (0u64..10)
                .map(|i| account_update(random_address(i + 100), (i + 1) * 1_000))
                .collect();
            let mt_root = run_merkleizer(updates.clone(), Arc::clone(&pool), Arc::clone(&provider));
            let st_root = run_oracle(updates);
            assert_eq!(mt_root, st_root, "10-account root mismatch");
        }

        // --- 100 accounts ---
        {
            let updates: Vec<AccountUpdate> = (0u64..100)
                .map(|i| account_update(random_address(i + 200), (i + 1) * 100))
                .collect();
            let mt_root = run_merkleizer(updates.clone(), Arc::clone(&pool), Arc::clone(&provider));
            let st_root = run_oracle(updates);
            assert_eq!(mt_root, st_root, "100-account root mismatch");
        }

        // --- SELFDESTRUCT (removed=true) ---
        {
            let addr = random_address(999);
            // First create the account.
            let create_updates = vec![account_update(addr, 50_000)];
            let mut state = BinaryTrieState::new();
            for u in &create_updates {
                state.apply_account_update(u).unwrap();
            }

            // Now remove it.
            let remove_update = AccountUpdate::removed(addr);
            state.apply_account_update(&remove_update).unwrap();
            let st_root = H256(state.state_root());

            let mut merkleizer = BinaryMerkleizer::new(
                H256::zero(),
                false,
                Arc::clone(&provider),
                Arc::clone(&pool),
            )
            .unwrap();
            merkleizer.feed_updates(create_updates).unwrap();
            merkleizer.feed_updates(vec![remove_update]).unwrap();
            let output = merkleizer.finalize().unwrap();

            assert_eq!(output.root, st_root, "SELFDESTRUCT root mismatch");
        }

        // --- code deployment ---
        {
            use bytes::Bytes;
            use ethrex_crypto::NativeCrypto;

            let addr = random_address(777);
            let bytecode = Bytes::from(vec![0x60u8, 0x00, 0x56]); // PUSH1 0x00 JUMP
            let code = Code::from_bytecode(bytecode, &NativeCrypto);
            let code_hash = code.hash;

            let mut update = AccountUpdate::new(addr);
            update.info = Some(AccountInfo {
                balance: U256::from(1000u64),
                nonce: 1,
                code_hash,
            });
            update.code = Some(code);

            let updates = vec![update];

            // Run through the full merkleizer and capture the output so we can
            // assert code_updates has exactly one entry (regression guard for
            // the pre-rewrite double-emit bug).
            let mut m = BinaryMerkleizer::new(
                H256::zero(),
                false,
                Arc::clone(&provider),
                Arc::clone(&pool),
            )
            .unwrap();
            m.feed_updates(updates.clone()).unwrap();
            let output = m.finalize().unwrap();
            let st_root = run_oracle(updates);
            assert_eq!(
                H256(output.root.0),
                st_root,
                "code deployment root mismatch"
            );
            assert_eq!(
                output.code_updates.len(),
                1,
                "single deployment must emit exactly one code_updates entry"
            );
            assert_eq!(
                output.code_updates[0].0, code_hash,
                "code_updates entry must be keyed by the declared code_hash"
            );
        }
    }

    fn run_merkleizer(
        updates: Vec<AccountUpdate>,
        pool: Arc<rayon::ThreadPool>,
        provider: Arc<dyn BinaryTrieProvider>,
    ) -> H256 {
        let mut m = BinaryMerkleizer::new(H256::zero(), false, provider, pool).unwrap();
        m.feed_updates(updates).unwrap();
        m.finalize().unwrap().root
    }

    fn run_oracle(updates: Vec<AccountUpdate>) -> H256 {
        let mut state = BinaryTrieState::new();
        for u in &updates {
            state.apply_account_update(u).unwrap();
        }
        H256(state.state_root())
    }

    // -----------------------------------------------------------------------
    // Task 4.15: sparse stem parity
    // -----------------------------------------------------------------------

    /// Verify that the sparse subtree merkelize in `merkle.rs` produces the same
    /// hash as the naive dense computation for StemNodes with occupancies
    /// 1, 2, 5, 128, and 256.
    ///
    /// We use the existing serial `state_root()` as the reference (it calls
    /// `compute_subtree_root` which now uses sparse recursion). This test verifies
    /// that the sparse result equals the dense naive result (computed here) so that
    /// the two paths agree bit-for-bit across all occupancy counts.
    #[test]
    fn test_sparse_stem_parity() {
        use crate::hash::blake3_hash;
        use crate::merkle::{ZERO_HASH, merkle_hash_64_pub};
        use crate::node::StemNode;
        use crate::node::{STEM_VALUES, SUBTREE_SIZE};
        use crate::node_store::NodeStore;

        // Dense reference: fill a 511-entry flat tree buffer and reduce it.
        fn dense_subtree(stem_node: &StemNode) -> [u8; 32] {
            let mut buf = [[0u8; 32]; SUBTREE_SIZE];
            for i in 0..STEM_VALUES {
                buf[255 + i] = ZERO_HASH;
            }
            for (&idx, val) in &stem_node.values {
                buf[255 + idx as usize] = blake3_hash(val);
            }
            for parent in (0..255usize).rev() {
                let left_child = 2 * parent + 1;
                let right_child = 2 * parent + 2;
                let mut hash_buf = [0u8; 64];
                hash_buf[..32].copy_from_slice(&buf[left_child]);
                hash_buf[32..].copy_from_slice(&buf[right_child]);
                buf[parent] = merkle_hash_64_pub(&hash_buf);
            }
            buf[0]
        }

        // Full StemNode hash (stem || 0x00 || subtree_root).
        fn dense_stem_hash(stem_node: &StemNode) -> [u8; 32] {
            let subtree = dense_subtree(stem_node);
            let mut buf = [0u8; 64];
            buf[..31].copy_from_slice(&stem_node.stem);
            buf[31] = 0x00;
            buf[32..].copy_from_slice(&subtree);
            merkle_hash_64_pub(&buf)
        }

        // Use the NodeStore path (same as hash_node_id) to get the sparse result.
        fn sparse_stem_hash_via_store(stem_node: StemNode) -> [u8; 32] {
            use crate::merkle::hash_node_id;
            use crate::node::{Node, SUBTREE_SIZE};
            let mut store = NodeStore::new_memory();
            let id = store.create(Node::Stem(stem_node));
            let mut buf = Box::new([[0u8; 32]; SUBTREE_SIZE]);
            hash_node_id(&mut store, id, &mut buf)
        }

        let stem = [0x42u8; 31];

        // occupancy 1
        {
            let mut sn = StemNode::new(stem);
            sn.set_value(7, [0xABu8; 32]);
            let dense = dense_stem_hash(&sn);
            let sparse = sparse_stem_hash_via_store(sn);
            assert_eq!(sparse, dense, "occupancy=1 mismatch");
        }

        // occupancy 2
        {
            let mut sn = StemNode::new(stem);
            sn.set_value(0, [0x11u8; 32]);
            sn.set_value(255, [0x22u8; 32]);
            let dense = dense_stem_hash(&sn);
            let sparse = sparse_stem_hash_via_store(sn);
            assert_eq!(sparse, dense, "occupancy=2 mismatch");
        }

        // occupancy 5
        {
            let mut sn = StemNode::new(stem);
            for i in [0u8, 1, 10, 100, 200] {
                sn.set_value(i, [i; 32]);
            }
            let dense = dense_stem_hash(&sn);
            let sparse = sparse_stem_hash_via_store(sn);
            assert_eq!(sparse, dense, "occupancy=5 mismatch");
        }

        // occupancy 128
        {
            let mut sn = StemNode::new(stem);
            for i in 0u8..128 {
                sn.set_value(i, [i; 32]);
            }
            let dense = dense_stem_hash(&sn);
            let sparse = sparse_stem_hash_via_store(sn);
            assert_eq!(sparse, dense, "occupancy=128 mismatch");
        }

        // occupancy 256 (full stem)
        {
            let mut sn = StemNode::new(stem);
            for i in 0u8..=255 {
                sn.set_value(i, [i; 32]);
            }
            // Also set value 255 explicitly (loop ends at 254 due to u8 wrap).
            let dense = dense_stem_hash(&sn);
            let sparse = sparse_stem_hash_via_store(sn);
            assert_eq!(sparse, dense, "occupancy=256 mismatch");
        }
    }

    // -----------------------------------------------------------------------
    // Task 4.16: micro-benchmark (ignored; run manually with --ignored)
    // -----------------------------------------------------------------------

    /// Micro-benchmark: 10k modified stems; compare serial `state_root()` vs
    /// `merkelize_parallel`. Record the ratio.
    ///
    /// Run with: `cargo test -p ethrex-binary-trie -- --ignored bench_merkelize_parallel`
    ///
    /// Target: ≥ 3× speedup on 8 cores. Not a gating test.
    #[test]
    #[ignore]
    fn bench_merkelize_parallel() {
        use std::time::Instant;

        let pool = Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(8)
                .build()
                .expect("pool"),
        );
        let provider = make_provider();

        // Build 10k account updates with distinct addresses.
        let updates: Vec<AccountUpdate> = (0u64..10_000)
            .map(|i| {
                let seed = i
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                account_update(random_address(seed), seed + 1)
            })
            .collect();

        // --- Serial path ---
        let mut serial_state = BinaryTrieState::new();
        for u in &updates {
            serial_state.apply_account_update(u).unwrap();
        }
        let t0 = Instant::now();
        let serial_root = serial_state.state_root();
        let serial_ms = t0.elapsed().as_micros();

        // --- Parallel path ---
        let mut parallel_state = BinaryTrieState::new();
        for u in &updates {
            parallel_state.apply_account_update(u).unwrap();
        }
        let t1 = Instant::now();
        let parallel_root = parallel_state.merkelize_parallel(&pool);
        let parallel_ms = t1.elapsed().as_micros();

        assert_eq!(
            serial_root, parallel_root,
            "parallel root must match serial oracle"
        );

        let ratio = serial_ms as f64 / parallel_ms as f64;
        eprintln!(
            "[bench_merkelize_parallel] serial={serial_ms}µs parallel={parallel_ms}µs ratio={ratio:.2}×"
        );

        // Also verify via full merkleizer pipeline.
        let mut merkleizer = BinaryMerkleizer::new(
            H256::zero(),
            false,
            Arc::clone(&provider),
            Arc::clone(&pool),
        )
        .unwrap();
        merkleizer.feed_updates(updates).unwrap();
        let output = merkleizer.finalize().unwrap();
        assert_eq!(
            output.root,
            H256(serial_root),
            "merkleizer output must match oracle"
        );
        eprintln!(
            "[bench_merkelize_parallel] merkleizer fkv_entries={}",
            match &output.node_updates {
                ethrex_state_backend::NodeUpdates::Binary { fkv_entries, .. } => fkv_entries.len(),
                _ => 0,
            }
        );
    }
}
