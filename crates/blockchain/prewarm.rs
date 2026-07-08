//! Mempool-driven state pre-warming.
//!
//! After each imported block, speculatively executes top-of-mempool
//! transactions against the new head state during the idle inter-slot gap,
//! refreshing as new transactions arrive until the next block lands or the
//! slot boundary passes. The warmth reaches the next block three ways: the
//! decoded per-slot `CachingDatabase` is handed to `execute_block_pipeline`
//! when the parent state and fork match (see `Blockchain::prewarmed`), the
//! reads populate the persistent caches underneath (RocksDB block cache,
//! code cache), and `warm_merkle_paths` walks the touched keys' trie paths
//! so the merkleizer finds interior nodes warm. Strictly read-only:
//! speculative execution results are discarded and never reach shared
//! state, so a wrong prediction costs wasted I/O, never incorrect state.

#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
use crate::PrewarmedEntry;
use crate::{Blockchain, BlockchainType};
use ethrex_common::types::{BlockHeader, MempoolTransaction, Transaction};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use rustc_hash::FxHashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
use tracing::info;
use tracing::warn;

/// Mainnet slot duration, used to compute the warming deadline (the next
/// slot boundary, one slot after the parent's timestamp).
const SLOT_DURATION_SECS: u64 = 12;

/// Warm up to this multiple of the parent block's gas limit worth of
/// mempool txs per warming snapshot (the initial pass and each delta).
#[cfg_attr(any(not(feature = "rayon"), feature = "eip-8025"), allow(dead_code))]
const GAS_BUDGET_MULTIPLIER: u64 = 6;

/// The next block's slot starts exactly one slot after its parent's; it cannot
/// arrive before that. Warming must end at this boundary.
fn next_slot_deadline_unix(parent_timestamp: u64) -> u64 {
    parent_timestamp.saturating_add(SLOT_DURATION_SECS)
}

/// Picks the warm set from a mempool snapshot: sender groups in nonce order,
/// groups ordered by their head tx's effective tip, accumulating until
/// `gas_budget` (sum of gas limits) is crossed — the crossing tx is included
/// and the group it belongs to is truncated there. Ordering fidelity does
/// not matter for warming, only membership; senders are never interleaved.
// This and the helpers below are only called from `run_pass`, which is
// compiled out when the rayon feature is disabled (or eip-8025 is active);
// keep them compiled for the unit tests instead of cfg-ing them out.
#[cfg_attr(any(not(feature = "rayon"), feature = "eip-8025"), allow(dead_code))]
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
/// their own predecessors), so warming them only inflates the warm volume.
#[cfg_attr(any(not(feature = "rayon"), feature = "eip-8025"), allow(dead_code))]
const MAX_WARMED_TXS_PER_SENDER_PER_SLOT: usize = 16;

/// Prepares a delta pass's warm set from a fresh mempool snapshot: truncates
/// each sender's queue (head-first, nonce order) to `cap`, then keeps only
/// senders whose truncated queue holds at least one not-yet-warmed tx (by
/// hash). Senders whose entire in-cap queue is already warmed this slot are
/// dropped.
///
/// The surviving senders keep their **full** in-cap queue, not just the new
/// suffix. This is required for warming to be effective: `warm_txs` rebuilds
/// each sender group from parent state on every pass, so a tx only warms the
/// state it touches if its nonce/balance predecessors run first. Replaying a
/// bare suffix (nonce N+1 without N) nonce-mismatches and warms nothing, and
/// still burns the cap — see [#6976]. Re-running an already-warmed prefix is
/// cheap: its reads are served from the shared `CachingDatabase`.
///
/// [#6976]: https://github.com/lambdaclass/ethrex/issues/6976
#[cfg_attr(any(not(feature = "rayon"), feature = "eip-8025"), allow(dead_code))]
fn select_dirty_senders(
    mut txs_by_sender: FxHashMap<Address, Vec<MempoolTransaction>>,
    warmed: &FxHashMap<H256, u64>,
    cap: usize,
) -> FxHashMap<Address, Vec<MempoolTransaction>> {
    txs_by_sender.retain(|_, txs| {
        txs.truncate(cap);
        txs.iter()
            .any(|mtx| !warmed.contains_key(&mtx.transaction().hash(&NativeCrypto)))
    });
    txs_by_sender
}

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
}

#[cfg_attr(any(not(feature = "rayon"), feature = "eip-8025"), allow(dead_code))]
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl PrewarmHandle {
    /// Fires the cancel flag of the most recently triggered pass. Called by
    /// the block executor before each import so in-flight warming stops
    /// competing with execution.
    pub fn cancel_current(&self) {
        if let Ok(flag) = self.current_cancel.lock() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Queues a warming pass for the child of `parent_header` (the newly
    /// imported head), installing a fresh cancel flag as the current one.
    pub fn trigger(&self, parent_header: BlockHeader) {
        let cancel = Arc::new(AtomicBool::new(false));
        if let Ok(mut current) = self.current_cancel.lock() {
            *current = cancel.clone();
        }
        let deadline_unix = next_slot_deadline_unix(parent_header.timestamp);
        let _ = self.sender.send(PrewarmRequest {
            parent_header,
            cancel,
            deadline_unix,
        });
    }
}

pub struct MempoolPrewarmer;

impl MempoolPrewarmer {
    /// Spawns the prewarmer worker. Prewarming is L1-only: returns `None`
    /// silently on L2. Returns `None` with a warn when the build lacks rayon
    /// (or has eip-8025 active), or the pool/worker-thread creation fails.
    pub fn spawn(blockchain: Arc<Blockchain>) -> Option<PrewarmHandle> {
        if !matches!(blockchain.options.r#type, BlockchainType::L1) {
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
            // Half the available cores: plenty for warming while leaving
            // headroom for the rest of the node during the idle window.
            let threads = (std::thread::available_parallelism().map_or(2, |n| n.get()) / 2).max(1);
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .thread_name(|i| format!("prewarm-{i}"))
                .build()
                .inspect_err(|e| warn!("Mempool prewarm disabled: pool creation failed: {e}"))
                .ok()?;
            let (sender, receiver) = mpsc::channel::<PrewarmRequest>();
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
                                run_pass(&blockchain, &pool, req)
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
            })
        }
    }
}

#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
fn run_pass(blockchain: &Blockchain, pool: &rayon::ThreadPool, req: PrewarmRequest) {
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

    // Predicted child base fee (same formula as the payload builder).
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
    let gas_budget = GAS_BUDGET_MULTIPLIER.saturating_mul(parent.gas_limit);

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
    header.timestamp = parent.timestamp.saturating_add(SLOT_DURATION_SECS);
    header.base_fee_per_gas = base_fee;
    header.gas_used = 0;
    if let Some(schedule) = config.get_fork_blob_schedule(header.timestamp) {
        let fork = config.fork(header.timestamp);
        header.excess_blob_gas = Some(calc_excess_blob_gas(parent, schedule, fork));
        header.blob_gas_used = Some(0);
    }

    // DB stack pinned at the parent (head) state, mirroring
    // execute_block_pipeline: StoreVmDatabase -> CachingDatabase. Reads
    // populate the persistent RocksDB block/code caches underneath; the
    // decoded layers here are throwaway.
    let vm_db = match StoreVmDatabase::new(blockchain.storage.clone(), parent.clone()) {
        Ok(vm_db) => vm_db,
        Err(e) => {
            warn!("Prewarm pass skipped: state db unavailable: {e}");
            return;
        }
    };
    let inner: Arc<dyn ethrex_vm::backends::LevmDatabase> =
        Arc::new(Box::new(vm_db) as ethrex_vm::DynVmDatabase);
    // Concrete handle kept for `touched_keys_where` (not on the trait); the
    // erased clone feeds `warm_txs` and the executor handoff.
    let cache = Arc::new(CachingDatabase::new(
        inner,
        blockchain.options.precompile_cache_enabled,
    ));
    let cache_dyn: Arc<dyn ethrex_vm::backends::LevmDatabase> = cache.clone();

    // Publish the cache for handoff: `execute_block_pipeline` seeds the next
    // block's execution with it when the parent hash and fork match (see
    // `PrewarmedCache`). Published at slot start so even a cut-short pass
    // hands over whatever it warmed; entries written by an in-flight delta
    // after cancellation are still valid parent-state reads.
    let warmed_fork = config.fork(header.timestamp);
    if let Ok(mut slot) = blockchain.prewarmed.0.lock() {
        *slot = Some(PrewarmedEntry {
            parent_hash: parent.hash(),
            fork: warmed_fork,
            cache: cache_dyn.clone(),
        });
    }

    let cancel = req.cancel.clone();
    let deadline = req.deadline_unix;
    let should_stop = move || cancel.load(Ordering::Relaxed) || unix_now() >= deadline;

    // Hashes (-> gas) of every tx warmed this slot. Drives `select_dirty_senders`
    // (a sender is re-warmed only while it still has a not-yet-warmed tx) and
    // the final tx/gas totals. Idempotent re-inserts from prefix replays don't
    // inflate it.
    let mut warmed_union: FxHashMap<H256, u64> = FxHashMap::default();
    // Slot-level dedup for merkle-path warming (see `warm_merkle_paths`).
    // Presence = account path already walked; the value keeps the hashed
    // address and storage root so later delta passes can open storage tries
    // for new slots of already-walked accounts without re-hashing.
    let mut walked_accounts: FxHashMap<Address, (H256, H256)> = FxHashMap::default();
    let mut walked_slots: rustc_hash::FxHashSet<(Address, H256)> = Default::default();
    let mut merkle_paths: u64 = 0;
    let mut passes: u32 = 0;
    let mut any_err = false;
    // `None` forces the first snapshot regardless of the counter's value.
    let mut last_seq: Option<u64> = None;

    // Refreshing delta passes: warm the initial snapshot, then keep warming
    // txs that arrive during the slot, until the next block arrives (cancel)
    // or the slot boundary (deadline); most included txs are sent moments
    // before their block, so a single start-of-slot pass would miss them.
    // All deltas share `cache`, so later passes get faster as the slot's
    // state accumulates.
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
            let fresh = select_dirty_senders(
                txs_by_sender,
                &warmed_union,
                MAX_WARMED_TXS_PER_SENDER_PER_SLOT,
            );
            let warm_set = select_warm_set(fresh, base_fee, gas_budget);
            if !warm_set.is_empty() {
                for (tx, _sender) in &warm_set {
                    warmed_union.insert(tx.hash(&NativeCrypto), tx.gas_limit());
                }
                // `warm_txs` takes borrowed transactions; bridge the owned set.
                let view: Vec<(&Transaction, Address)> =
                    warm_set.iter().map(|(tx, s)| (tx, *s)).collect();
                let result = pool.install(|| {
                    LEVM::warm_txs(
                        &view,
                        &header,
                        cache_dyn.clone(),
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
                    &cache,
                    &mut walked_accounts,
                    &mut walked_slots,
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
    cache: &ethrex_vm::backends::CachingDatabase,
    walked_accounts: &mut FxHashMap<Address, (H256, H256)>,
    walked_slots: &mut rustc_hash::FxHashSet<(Address, H256)>,
    should_stop: &(dyn Fn() -> bool + Sync),
) -> u64 {
    use ethrex_common::constants::EMPTY_TRIE_HASH;
    use ethrex_common::utils::keccak;
    use ethrex_storage::hash_key;
    use tracing::debug;

    // Collect only the delta: keys not walked in an earlier pass this slot.
    // The cache grows monotonically, so filtering here keeps the per-pass
    // allocation O(new) instead of re-cloning the whole accumulated set.
    let touched = cache
        .touched_keys_where(&|addr| !walked_accounts.contains_key(addr), &|slot_key| {
            !walked_slots.contains(slot_key)
        });
    let mut walked: u64 = 0;

    let state_trie = match blockchain.storage.open_state_trie(parent.state_root) {
        Ok(trie) => trie,
        Err(e) => {
            debug!("Merkle warm skipped: state trie unavailable: {e}");
            return 0;
        }
    };

    for (addr, storage_root) in touched.accounts {
        let hashed = keccak(addr.to_fixed_bytes());
        walked_accounts.insert(addr, (hashed, storage_root));
        if should_stop() {
            return walked;
        }
        let _ = state_trie.get_proof(hashed.as_bytes());
        walked += 1;
    }

    // Group new slots per account so each storage trie is opened once.
    let mut by_account: FxHashMap<Address, Vec<H256>> = FxHashMap::default();
    for (addr, key) in touched.slots {
        if walked_slots.insert((addr, key)) {
            by_account.entry(addr).or_default().push(key);
        }
    }
    for (addr, keys) in by_account {
        if should_stop() {
            return walked;
        }
        // Entries persist across the slot's passes, so slots of accounts
        // walked in earlier deltas still resolve.
        let Some((hashed, storage_root)) = walked_accounts.get(&addr).copied() else {
            continue;
        };
        if storage_root == *EMPTY_TRIE_HASH {
            continue;
        }
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
    fn select_dirty_senders_truncates_to_cap_head_first() {
        let (a, a0) = make_tx(0xaa, 0, 100, 50, 21_000);
        let (_, a1) = make_tx(0xaa, 1, 100, 50, 21_000);
        let (_, a2) = make_tx(0xaa, 2, 100, 50, 21_000);
        let mut map = FxHashMap::default();
        map.insert(a, vec![a0, a1, a2]);
        // Nothing warmed yet: sender is dirty, kept and truncated to cap 2.
        let out = select_dirty_senders(map, &FxHashMap::default(), 2);
        assert_eq!(out[&a].len(), 2);
        assert_eq!(out[&a][0].transaction().nonce(), 0); // head kept, nonce order preserved
        assert_eq!(out[&a][1].transaction().nonce(), 1);
    }

    #[test]
    fn select_dirty_senders_keeps_full_queue_when_prefix_warmed() {
        use ethrex_crypto::NativeCrypto;
        // Sender a: nonce 0 already warmed, nonce 1 is new -> a is dirty and
        // must keep its FULL queue (0 and 1) so nonce 1 gets its predecessor.
        let (a, a0) = make_tx(0xaa, 0, 100, 50, 21_000);
        let (_, a1) = make_tx(0xaa, 1, 100, 50, 21_000);
        // Sender b: only tx fully warmed -> dropped entirely.
        let (b, b0) = make_tx(0xbb, 0, 100, 50, 21_000);
        let mut warmed = FxHashMap::default();
        warmed.insert(a0.transaction().hash(&NativeCrypto), 21_000u64);
        warmed.insert(b0.transaction().hash(&NativeCrypto), 21_000u64);
        let mut map = FxHashMap::default();
        map.insert(a, vec![a0, a1]);
        map.insert(b, vec![b0]);
        let fresh = select_dirty_senders(map, &warmed, 16);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[&a].len(), 2); // full queue retained, prefix included
        assert_eq!(fresh[&a][0].transaction().nonce(), 0);
        assert_eq!(fresh[&a][1].transaction().nonce(), 1);
    }

    #[test]
    fn select_dirty_senders_drops_when_new_tx_is_beyond_cap() {
        use ethrex_crypto::NativeCrypto;
        // First two nonces warmed; the only new tx sits beyond the cap, so
        // after truncation the in-cap queue is fully warmed -> dropped.
        let (a, a0) = make_tx(0xaa, 0, 100, 50, 21_000);
        let (_, a1) = make_tx(0xaa, 1, 100, 50, 21_000);
        let (_, a2) = make_tx(0xaa, 2, 100, 50, 21_000);
        let mut warmed = FxHashMap::default();
        warmed.insert(a0.transaction().hash(&NativeCrypto), 21_000u64);
        warmed.insert(a1.transaction().hash(&NativeCrypto), 21_000u64);
        let mut map = FxHashMap::default();
        map.insert(a, vec![a0, a1, a2]);
        let fresh = select_dirty_senders(map, &warmed, 2);
        assert!(fresh.is_empty());
    }

    #[test]
    fn deadline_is_next_slot_boundary() {
        assert_eq!(
            next_slot_deadline_unix(1_700_000_000),
            1_700_000_000 + SLOT_DURATION_SECS
        );
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
