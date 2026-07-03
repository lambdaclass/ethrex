//! Mempool-driven state pre-warming (PoC).
//!
//! After each imported block, speculatively executes top-of-mempool
//! transactions against the new head state during the idle inter-slot gap.
//! The warmth reaches the next block three ways: the decoded per-slot
//! `CachingDatabase` is handed to `execute_block_pipeline` when the parent
//! state and fork match (see `Blockchain::prewarmed`), the reads populate
//! the persistent caches underneath (RocksDB block cache, code cache), and
//! `warm_merkle_paths` walks the touched keys' trie paths so the merkleizer
//! finds interior nodes warm. Strictly read-only: speculative execution
//! results are discarded and never reach shared state, so a wrong prediction
//! costs wasted I/O, never incorrect state.
//! The pass refreshes as new transactions arrive (per-sender depth capped by
//! `MAX_WARMED_TXS_PER_SENDER_PER_SLOT`), covering most of the inter-block
//! window (late-arriving txs dominate the next block's content); it is
//! cancelled the moment the next block arrives and never runs past the next
//! slot boundary.

use ethrex_common::types::{MempoolTransaction, Transaction};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use rustc_hash::FxHashMap;

/// The next block's slot starts exactly one slot after its parent's; it cannot
/// arrive before that. Warming must end at this boundary.
fn next_slot_deadline_unix(parent_timestamp: u64, slot_duration_secs: u64) -> u64 {
    parent_timestamp.saturating_add(slot_duration_secs)
}

/// Picks the warm set from a mempool snapshot: sender groups in nonce order,
/// groups ordered by their head tx's effective tip, accumulating until
/// `gas_budget` (sum of gas limits) is crossed — the crossing tx is included
/// and the group it belongs to is truncated there. Ordering fidelity does
/// not matter for warming, only membership; senders are never interleaved.
fn select_warm_set(
    txs_by_sender: FxHashMap<Address, Vec<MempoolTransaction>>,
    base_fee: Option<u64>,
    gas_budget: u64,
) -> Vec<(Transaction, Address)> {
    let mut groups: Vec<(U256, Address, Vec<MempoolTransaction>)> = txs_by_sender
        .into_iter()
        .filter_map(|(sender, txs)| {
            let tip = txs.first()?.transaction().effective_gas_tip(base_fee)?;
            Some((tip, sender, txs))
        })
        .collect();
    groups.sort_by(|a, b| b.0.cmp(&a.0));

    let mut out = Vec::new();
    let mut gas_acc: u64 = 0;
    'outer: for (_, sender, txs) in groups {
        for mtx in txs {
            gas_acc = gas_acc.saturating_add(mtx.transaction().gas_limit());
            out.push((mtx.transaction().clone(), sender));
            if gas_acc >= gas_budget {
                break 'outer;
            }
        }
    }
    out
}

/// Per-slot cap on warmed txs per sender. Deep same-sender queues beyond
/// this depth cannot realistically land in the next block (they sit behind
/// their own predecessors), so warming them only inflates the warm volume,
/// which we measured live as a small next-block throughput penalty.
/// Generous enough to keep legitimate batchers (exchanges land dozens-deep runs, rarely
/// more) while cutting off hundred-deep spam queues.
pub(crate) const MAX_WARMED_TXS_PER_SENDER_PER_SLOT: usize = 16;

/// Enforces [`MAX_WARMED_TXS_PER_SENDER_PER_SLOT`] across a slot's delta
/// passes: `warmed_per_sender` counts txs already warmed this slot, and each
/// sender's snapshot group is truncated to the remaining allowance. Nonce
/// order within groups is preserved; exhausted senders are removed.
fn cap_sender_depth(
    mut txs_by_sender: FxHashMap<Address, Vec<MempoolTransaction>>,
    warmed_per_sender: &FxHashMap<Address, usize>,
    cap: usize,
) -> FxHashMap<Address, Vec<MempoolTransaction>> {
    txs_by_sender.retain(|sender, txs| {
        let used = warmed_per_sender.get(sender).copied().unwrap_or(0);
        let allowance = cap.saturating_sub(used);
        txs.truncate(allowance);
        !txs.is_empty()
    });
    txs_by_sender
}

/// Drops txs already warmed this slot (by hash) from a fresh mempool
/// snapshot, so delta passes only warm new arrivals. Nonce order within the
/// surviving sender groups is preserved; senders left empty are removed.
fn drop_already_warmed(
    mut txs_by_sender: FxHashMap<Address, Vec<MempoolTransaction>>,
    warmed: &FxHashMap<H256, u64>,
) -> FxHashMap<Address, Vec<MempoolTransaction>> {
    if warmed.is_empty() {
        return txs_by_sender;
    }
    txs_by_sender.retain(|_, txs| {
        txs.retain(|mtx| !warmed.contains_key(&mtx.transaction().hash(&NativeCrypto)));
        !txs.is_empty()
    });
    txs_by_sender
}

#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
use crate::MempoolPrewarmOptions;
use crate::{Blockchain, BlockchainType};
use ethrex_common::types::BlockHeader;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

// Fields are only read by `run_pass`, which is compiled out when the rayon
// feature is disabled (or eip-8025 is active); avoid a dead-code warning in
// that configuration, where the fields are still written by `trigger`.
#[cfg_attr(any(not(feature = "rayon"), feature = "eip-8025"), allow(dead_code))]
struct PrewarmRequest {
    parent_header: BlockHeader,
    cancel: Arc<AtomicBool>,
    deadline_unix: u64,
}

pub struct PrewarmHandle {
    sender: mpsc::Sender<PrewarmRequest>,
    /// Cancel flag of the most recently triggered pass. `trigger` replaces
    /// it, `cancel_current` fires it. Both are called only from the single
    /// block-executor thread, so replace/fire cannot race.
    current_cancel: Mutex<Arc<AtomicBool>>,
    slot_duration_secs: u64,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl PrewarmHandle {
    pub fn cancel_current(&self) {
        if let Ok(flag) = self.current_cancel.lock() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    pub fn trigger(&self, parent_header: BlockHeader) {
        let cancel = Arc::new(AtomicBool::new(false));
        if let Ok(mut current) = self.current_cancel.lock() {
            *current = cancel.clone();
        }
        let deadline_unix =
            next_slot_deadline_unix(parent_header.timestamp, self.slot_duration_secs);
        let _ = self.sender.send(PrewarmRequest {
            parent_header,
            cancel,
            deadline_unix,
        });
    }
}

pub struct MempoolPrewarmer;

impl MempoolPrewarmer {
    /// Spawns the prewarmer worker. Returns `None` silently when the feature
    /// is disabled; returns `None` with a warn when the chain is L2, the
    /// build lacks rayon (or has eip-8025 active), or the pool/worker-thread
    /// creation fails.
    pub fn spawn(blockchain: Arc<Blockchain>) -> Option<PrewarmHandle> {
        let opts = blockchain.options.mempool_prewarm.clone();
        if !opts.enabled {
            return None;
        }
        if !matches!(blockchain.options.r#type, BlockchainType::L1) {
            warn!("Mempool prewarm is L1-only; disabled");
            return None;
        }
        #[cfg(any(not(feature = "rayon"), feature = "eip-8025"))]
        {
            warn!(
                "Mempool prewarm requires the rayon feature and is unavailable on eip-8025 builds; disabled"
            );
            None
        }
        #[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
        {
            let threads = if opts.num_threads == 0 {
                (std::thread::available_parallelism().map_or(2, |n| n.get()) / 2).max(1)
            } else {
                opts.num_threads
            };
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .thread_name(|i| format!("prewarm-{i}"))
                .build()
                .inspect_err(|e| warn!("Mempool prewarm disabled: pool creation failed: {e}"))
                .ok()?;
            let (sender, receiver) = mpsc::channel::<PrewarmRequest>();
            let slot_duration_secs = opts.slot_duration_secs;
            std::thread::Builder::new()
                .name("mempool_prewarmer".to_string())
                .spawn(move || {
                    while let Ok(mut req) = receiver.recv() {
                        // Drain to the newest request; stale ones are already cancelled.
                        while let Ok(next) = receiver.try_recv() {
                            req = next;
                        }
                        // A panic inside a pass (speculating on arbitrary
                        // mempool txs) must not kill this loop: the worker is
                        // the feature — if it dies, every later trigger sends
                        // to a dropped receiver and prewarming silently
                        // disables itself for the rest of the process.
                        let outcome =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                run_pass(&blockchain, &pool, req, &opts)
                            }));
                        if outcome.is_err() {
                            warn!("Prewarm pass panicked; worker continuing");
                        }
                    }
                })
                .inspect_err(|e| warn!("Mempool prewarm disabled: worker spawn failed: {e}"))
                .ok()?;
            Some(PrewarmHandle {
                sender,
                current_cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
                slot_duration_secs,
            })
        }
    }
}

#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
fn run_pass(
    blockchain: &Blockchain,
    pool: &rayon::ThreadPool,
    req: PrewarmRequest,
    opts: &MempoolPrewarmOptions,
) {
    use crate::mempool::PendingTxFilter;
    use crate::vm::StoreVmDatabase;
    use ethrex_common::types::{
        ELASTICITY_MULTIPLIER, calc_excess_blob_gas, calculate_base_fee_per_gas,
    };
    use ethrex_vm::backends::CachingDatabase;
    use ethrex_vm::backends::VMType;
    use ethrex_vm::backends::levm::LEVM;
    use tracing::debug;

    if req.cancel.load(Ordering::Relaxed) || unix_now() >= req.deadline_unix {
        debug!(
            "Prewarm pass for child of block {} skipped: stale at start",
            req.parent_header.number
        );
        return;
    }

    let start = Instant::now();
    let parent = &req.parent_header;

    // Predicted child base fee (same formula as the payload builder,
    // crates/blockchain/payload.rs:173).
    let base_fee = calculate_base_fee_per_gas(
        parent.gas_limit,
        parent.gas_limit,
        parent.gas_used,
        parent.base_fee_per_gas.unwrap_or_default(),
        ELASTICITY_MULTIPLIER,
    );

    // Mempool snapshot filter. blob_fee: None on purpose — blob txs are
    // warmed like any other tx (the sidecar is not state), no fee gate needed.
    let filter = PendingTxFilter {
        base_fee,
        ..Default::default()
    };
    // Budget per warming snapshot; delta snapshots each get fresh headroom
    // (their size is naturally bounded by the slot's arrival rate).
    let gas_budget = opts.gas_budget_multiplier.saturating_mul(parent.gas_limit);

    // Synthetic child header: clone the parent and override what the warm
    // path reads (fork selection by timestamp, base fee, blob fee inputs).
    // Approximation is fine — warming only needs plausible execution context.
    let config = blockchain.storage.get_chain_config();
    let mut header = parent.clone();
    // The clone carries the parent's cached hash; reset so any future
    // consumer recomputes it for the (different) synthetic child header.
    header.hash = Default::default();
    header.parent_hash = parent.hash();
    header.number = parent.number.saturating_add(1);
    header.timestamp = parent.timestamp.saturating_add(opts.slot_duration_secs);
    header.base_fee_per_gas = base_fee;
    header.gas_used = 0;
    if let Some(schedule) = config.get_fork_blob_schedule(header.timestamp) {
        let fork = config.fork(header.timestamp);
        header.excess_blob_gas = Some(calc_excess_blob_gas(parent, schedule, fork));
        header.blob_gas_used = Some(0);
    }

    // DB stack pinned at the parent (head) state, mirroring
    // execute_block_pipeline (blockchain.rs:548) + Evm::new_from_db_for_l1:
    // StoreVmDatabase -> CachingDatabase. Reads populate the persistent
    // RocksDB block cache and code cache underneath; the decoded layers here
    // are throwaway.
    let vm_db = match StoreVmDatabase::new(blockchain.storage.clone(), parent.clone()) {
        Ok(vm_db) => vm_db,
        Err(e) => {
            warn!("Prewarm pass skipped: state db unavailable: {e}");
            return;
        }
    };
    let inner: Arc<dyn ethrex_vm::backends::LevmDatabase> =
        Arc::new(Box::new(vm_db) as ethrex_vm::DynVmDatabase);
    // Concrete handle kept for `touched_keys` (not on the trait); the erased
    // clone feeds `warm_txs` and the executor handoff.
    let caching_concrete = Arc::new(CachingDatabase::new(
        inner,
        blockchain.options.precompile_cache_enabled,
    ));
    let caching: Arc<dyn ethrex_vm::backends::LevmDatabase> = caching_concrete.clone();

    // Publish the cache for handoff: `execute_block_pipeline` seeds the next
    // block's execution with it when the parent hash AND fork match (the
    // fork guards the fork-dependent precompile-cache layer across missed
    // slots that span a fork activation). Published at slot start so even a
    // cut-short pass hands over whatever it warmed; entries written by an
    // in-flight delta after cancellation are still valid parent-state reads
    // going into the same shared instance.
    let warmed_fork = config.fork(header.timestamp);
    if let Ok(mut p) = blockchain.prewarmed.0.lock() {
        *p = Some((parent.hash(), warmed_fork, caching.clone()));
    }

    let cancel = req.cancel.clone();
    let deadline = req.deadline_unix;
    let should_stop = move || cancel.load(Ordering::Relaxed) || unix_now() >= deadline;

    let mut warmed_union: FxHashMap<H256, u64> = FxHashMap::default();
    // Slot-level dedup for merkle-path warming (see `warm_merkle_paths`).
    // Presence in the map = account path already walked; the value keeps the
    // storage root so later delta passes can open storage tries for new
    // slots of already-walked accounts.
    let mut merkled_roots: FxHashMap<Address, H256> = FxHashMap::default();
    let mut merkled_slots: rustc_hash::FxHashSet<(Address, H256)> = Default::default();
    let mut merkle_paths: u64 = 0;
    // Slot-level per-sender counts backing the depth cap: a per-snapshot cap
    // alone would leak — once a sender's head txs are warmed and dropped by
    // `drop_already_warmed`, the next delta would see the queue's tail as a
    // fresh head and keep digging.
    let mut warmed_per_sender: FxHashMap<Address, usize> = FxHashMap::default();
    let mut passes: u32 = 0;
    let mut any_err = false;
    // `None` forces the first snapshot regardless of the counter's value.
    let mut last_seq: Option<u64> = None;

    // Refreshing delta passes: warm the initial snapshot, then keep warming
    // txs that arrive during the slot, until the next block arrives (cancel)
    // or the slot boundary (deadline). This covers most of the inter-block
    // window instead of one instant at its start, which matters because most
    // included txs are sent moments before their block: live measurement
    // during development (see the PR) found roughly half of block gas
    // reached the mempool only after a start-of-slot snapshot. All deltas
    // share `caching`, so later passes get faster as the slot's state
    // accumulates.
    loop {
        if should_stop() {
            break;
        }
        let seq = blockchain.mempool.tx_seq();
        if last_seq != Some(seq) {
            last_seq = Some(seq);
            let txs_by_sender = match blockchain.mempool.filter_transactions(&filter) {
                Ok(txs_by_sender) => txs_by_sender,
                Err(e) => {
                    warn!("Prewarm pass aborted: mempool snapshot failed: {e}");
                    break;
                }
            };
            let fresh = drop_already_warmed(txs_by_sender, &warmed_union);
            let fresh = cap_sender_depth(
                fresh,
                &warmed_per_sender,
                MAX_WARMED_TXS_PER_SENDER_PER_SLOT,
            );
            let warm_set = select_warm_set(fresh, base_fee, gas_budget);
            if !warm_set.is_empty() {
                for (tx, sender) in &warm_set {
                    warmed_union.insert(tx.hash(&NativeCrypto), tx.gas_limit());
                    *warmed_per_sender.entry(*sender).or_default() += 1;
                }
                // `warm_txs` takes borrowed transactions; bridge the owned set.
                let view: Vec<(&Transaction, Address)> =
                    warm_set.iter().map(|(tx, s)| (tx, *s)).collect();
                let result = pool.install(|| {
                    LEVM::warm_txs(
                        &view,
                        &header,
                        caching.clone(),
                        VMType::L1,
                        &NativeCrypto,
                        &should_stop,
                    )
                });
                any_err |= result.is_err();
                passes += 1;
                merkle_paths += warm_merkle_paths(
                    blockchain,
                    parent,
                    &caching_concrete,
                    &mut merkled_roots,
                    &mut merkled_slots,
                    &should_stop,
                );
            }
        }
        // Arrival-poll cadence. Sleeping burns no CPU; the only cost is up to
        // one interval of harmless lag on cancel (no work is in flight).
        std::thread::sleep(Duration::from_millis(100));
    }

    if passes == 0 {
        debug!("Prewarm pass: nothing to warm this slot");
        return;
    }

    let stop_reason = if req.cancel.load(Ordering::Relaxed) {
        "cancelled"
    } else if unix_now() >= deadline {
        "deadline"
    } else {
        // The refresh loop only exits early on a snapshot error (warned above).
        "aborted"
    };
    let total_gas: u64 = warmed_union.values().sum();
    info!(
        "Prewarm pass for block {}: {} txs, {} gas, passes={}, merkle_paths={}, {:?}, stop={}, err={}",
        header.number,
        warmed_union.len(),
        total_gas,
        passes,
        merkle_paths,
        start.elapsed(),
        stop_reason,
        any_err,
    );
}

/// Walks the account- and storage-trie paths of every key the warming pass
/// has touched so far, via `get_proof` (which reads each interior node on the
/// path). Ordinary reads skip interior trie nodes once flat-key-value is
/// active, so without this the merkleizer finds them cold at block time; the
/// proof walks pull them into the RocksDB block cache during the idle window.
/// Proof outputs are discarded — the reads are the product. Returns the
/// number of newly walked paths.
#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
fn warm_merkle_paths(
    blockchain: &Blockchain,
    parent: &BlockHeader,
    caching: &ethrex_vm::backends::CachingDatabase,
    merkled_roots: &mut FxHashMap<Address, H256>,
    done_slots: &mut rustc_hash::FxHashSet<(Address, H256)>,
    should_stop: &(dyn Fn() -> bool + Sync),
) -> u64 {
    use ethrex_common::constants::EMPTY_TRIE_HASH;
    use ethrex_crypto::keccak::keccak_hash;
    use ethrex_storage::hash_key;
    use tracing::debug;

    // Collect only the delta: keys not walked in an earlier pass this slot.
    // The cache grows monotonically, so filtering here keeps the per-pass
    // allocation O(new) instead of re-cloning the whole accumulated set.
    let (accounts, slots) = caching
        .touched_keys_where(&|addr| !merkled_roots.contains_key(addr), &|slot_key| {
            !done_slots.contains(slot_key)
        });
    let mut walked: u64 = 0;

    let state_trie = match blockchain.storage.open_state_trie(parent.state_root) {
        Ok(trie) => trie,
        Err(e) => {
            debug!("Merkle warm skipped: state trie unavailable: {e}");
            return 0;
        }
    };

    for (addr, storage_root) in accounts {
        merkled_roots.insert(addr, storage_root);
        if should_stop() {
            return walked;
        }
        let hashed = H256::from(keccak_hash(addr.to_fixed_bytes()));
        let _ = state_trie.get_proof(hashed.as_bytes());
        walked += 1;
    }

    // Group new slots per account so each storage trie is opened once.
    let mut by_account: FxHashMap<Address, Vec<H256>> = FxHashMap::default();
    for (addr, key) in slots {
        if done_slots.insert((addr, key)) {
            by_account.entry(addr).or_default().push(key);
        }
    }
    for (addr, keys) in by_account {
        if should_stop() {
            return walked;
        }
        // Roots persist across the slot's passes, so slots of accounts
        // walked in earlier deltas still resolve.
        let Some(storage_root) = merkled_roots.get(&addr).copied() else {
            continue;
        };
        if storage_root == *EMPTY_TRIE_HASH {
            continue;
        }
        let hashed = H256::from(keccak_hash(addr.to_fixed_bytes()));
        let Ok(storage_trie) =
            blockchain
                .storage
                .open_storage_trie(hashed, parent.state_root, storage_root)
        else {
            continue;
        };
        for key in keys {
            if should_stop() {
                return walked;
            }
            let _ = storage_trie.get_proof(&hash_key(&key));
            walked += 1;
        }
    }
    walked
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::Address;
    use ethrex_common::types::{EIP1559Transaction, MempoolTransaction, Transaction};
    use rustc_hash::FxHashMap;

    fn make_tx(
        sender_byte: u8,
        nonce: u64,
        max_fee: u64,
        tip: u64,
        gas_limit: u64,
    ) -> (Address, MempoolTransaction) {
        let sender = Address::repeat_byte(sender_byte);
        let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            nonce,
            gas_limit,
            max_fee_per_gas: max_fee,
            max_priority_fee_per_gas: tip,
            ..Default::default()
        });
        (sender, MempoolTransaction::new(tx, sender))
    }

    #[test]
    fn cap_sender_depth_truncates_and_respects_slot_counts() {
        let (a, a0) = make_tx(0xaa, 0, 100, 50, 21_000);
        let (_, a1) = make_tx(0xaa, 1, 100, 50, 21_000);
        let (_, a2) = make_tx(0xaa, 2, 100, 50, 21_000);
        let (b, b0) = make_tx(0xbb, 0, 100, 50, 21_000);
        let mut map = FxHashMap::default();
        map.insert(a, vec![a0, a1, a2]);
        map.insert(b, vec![b0]);
        // Sender a already warmed 1 tx this slot; cap 2 leaves allowance 1.
        let mut counts = FxHashMap::default();
        counts.insert(a, 1usize);
        let capped = cap_sender_depth(map, &counts, 2);
        assert_eq!(capped[&a].len(), 1);
        assert_eq!(capped[&a][0].transaction().nonce(), 0); // head kept, nonce order preserved
        assert_eq!(capped[&b].len(), 1); // untouched sender unaffected
        // Exhausted sender is removed entirely.
        let mut counts2 = FxHashMap::default();
        counts2.insert(b, 2usize);
        let mut map2 = FxHashMap::default();
        let (_, b1) = make_tx(0xbb, 1, 100, 50, 21_000);
        map2.insert(b, vec![b1]);
        let capped2 = cap_sender_depth(map2, &counts2, 2);
        assert!(capped2.is_empty());
    }

    #[test]
    fn drop_already_warmed_removes_by_hash_and_empty_senders() {
        use ethrex_crypto::NativeCrypto;
        let (a, tx_a0) = make_tx(0xaa, 0, 100, 50, 21_000);
        let (_, tx_a1) = make_tx(0xaa, 1, 100, 50, 21_000);
        let (b, tx_b0) = make_tx(0xbb, 0, 100, 50, 21_000);
        let mut warmed = FxHashMap::default();
        warmed.insert(tx_a0.transaction().hash(&NativeCrypto), 21_000u64);
        warmed.insert(tx_b0.transaction().hash(&NativeCrypto), 21_000u64);
        let mut map = FxHashMap::default();
        map.insert(a, vec![tx_a0, tx_a1]);
        map.insert(b, vec![tx_b0]);
        let fresh = drop_already_warmed(map, &warmed);
        // Sender b fully warmed -> removed; sender a keeps only nonce 1.
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[&a].len(), 1);
        assert_eq!(fresh[&a][0].transaction().nonce(), 1);
    }

    #[test]
    fn deadline_is_next_slot_boundary() {
        assert_eq!(next_slot_deadline_unix(1_700_000_000, 12), 1_700_000_012);
    }

    #[test]
    fn select_prefers_higher_tip_sender_and_respects_budget() {
        let (a, tx_a) = make_tx(0xaa, 0, 100, 50, 21_000); // high tip
        let (b, tx_b) = make_tx(0xbb, 0, 100, 10, 21_000); // low tip
        let mut map = FxHashMap::default();
        map.insert(a, vec![tx_a]);
        map.insert(b, vec![tx_b]);
        // Budget only fits one tx: the high-tip sender must win.
        let set = select_warm_set(map, Some(1), 21_000);
        assert_eq!(set.len(), 1);
        assert_eq!(set[0].1, a);
    }

    #[test]
    fn select_keeps_nonce_order_within_sender() {
        let (a, tx0) = make_tx(0xaa, 0, 100, 50, 21_000);
        let (_, tx1) = make_tx(0xaa, 1, 100, 50, 21_000);
        let mut map = FxHashMap::default();
        map.insert(a, vec![tx0, tx1]);
        let set = select_warm_set(map, Some(1), 1_000_000);
        assert_eq!(set.len(), 2);
        assert_eq!(set[0].0.nonce(), 0);
        assert_eq!(set[1].0.nonce(), 1);
    }

    #[test]
    fn select_includes_budget_crossing_tx_then_stops() {
        let (a, tx0) = make_tx(0xaa, 0, 100, 50, 30_000);
        let (_, tx1) = make_tx(0xaa, 1, 100, 50, 30_000);
        let (_, tx2) = make_tx(0xaa, 2, 100, 50, 30_000);
        let mut map = FxHashMap::default();
        map.insert(a, vec![tx0, tx1, tx2]);
        // Budget 40k: tx0 (30k) is under, tx1 crosses to 60k and is included, tx2 is not.
        let set = select_warm_set(map, Some(1), 40_000);
        assert_eq!(set.len(), 2);
    }
}
