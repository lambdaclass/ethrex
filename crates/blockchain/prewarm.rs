//! Mempool-driven state pre-warming (PoC).
//!
//! After each imported block, speculatively executes top-of-mempool
//! transactions against the new head state during the idle inter-slot gap, so
//! the next block's state reads hit warm persistent caches (RocksDB block
//! cache, code cache). Read-only and throwaway: speculative results are
//! discarded; a wrong prediction costs wasted I/O, never incorrect state.
//! The pass is cancelled the moment the next block arrives and never runs
//! past the next slot boundary.

use ethrex_common::types::{Block, MempoolTransaction, Transaction};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use rustc_hash::FxHashMap;

/// The next block's slot starts exactly one slot after its parent's; it cannot
/// arrive before that. Warming must end at this boundary.
pub fn next_slot_deadline_unix(parent_timestamp: u64, slot_duration_secs: u64) -> u64 {
    parent_timestamp.saturating_add(slot_duration_secs)
}

/// Picks the warm set from a mempool snapshot: whole sender groups (nonce
/// order preserved), groups ordered by their head tx's effective tip, until
/// `gas_budget` (sum of gas limits) is crossed. Ordering fidelity does not
/// matter for warming, only membership — hence whole groups, no interleaving.
pub fn select_warm_set(
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

/// How much of an arrived block the last warming pass covered. `warmed` maps
/// warmed tx hash -> gas limit.
#[derive(Debug)]
pub struct OverlapStats {
    pub matched_txs: usize,
    pub block_txs: usize,
    pub matched_gas: u64,
    pub block_gas: u64,
}

pub fn compute_overlap(warmed: &FxHashMap<H256, u64>, block: &Block) -> OverlapStats {
    let mut stats = OverlapStats {
        matched_txs: 0,
        block_txs: block.body.transactions.len(),
        matched_gas: 0,
        block_gas: 0,
    };
    for tx in &block.body.transactions {
        let gas = tx.gas_limit();
        stats.block_gas = stats.block_gas.saturating_add(gas);
        if warmed.contains_key(&tx.hash(&NativeCrypto)) {
            stats.matched_txs += 1;
            stats.matched_gas = stats.matched_gas.saturating_add(gas);
        }
    }
    stats
}

#[cfg(all(feature = "rayon", not(feature = "eip-8025")))]
use crate::MempoolPrewarmOptions;
use crate::{Blockchain, BlockchainType};
use ethrex_common::types::BlockHeader;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{SystemTime, UNIX_EPOCH};
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
    /// Cancel flag of the most recently triggered pass. `trigger` replaces it,
    /// `cancel_current` sets it. Both are called only from the single
    /// block-executor thread, so replace/set cannot race.
    current_cancel: Mutex<Arc<AtomicBool>>,
    /// hash -> gas_limit of the last pass's warm set, for the overlap metric.
    last_warmed: Arc<Mutex<Option<FxHashMap<H256, u64>>>>,
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

    /// Called on every newPayload arrival, before execution. Telemetry only.
    pub fn log_block_arrival(&self, block: &Block) {
        // Propagation offset: how far into its slot the block reached us.
        // Confirms the real margin the slot-boundary deadline leaves.
        let offset_secs = unix_now().saturating_sub(block.header.timestamp);
        let overlap = self
            .last_warmed
            .lock()
            .ok()
            .and_then(|mut w| w.take())
            .map(|warmed| compute_overlap(&warmed, block));
        match overlap {
            Some(s) => info!(
                "prewarm arrival: block {} offset {}s, overlap {}/{} txs {}/{} gas",
                block.header.number,
                offset_secs,
                s.matched_txs,
                s.block_txs,
                s.matched_gas,
                s.block_gas
            ),
            None => info!(
                "prewarm arrival: block {} offset {}s, no warm set",
                block.header.number, offset_secs
            ),
        }
    }
}

pub struct MempoolPrewarmer;

impl MempoolPrewarmer {
    /// Spawns the prewarmer worker. Returns `None` silently when the feature
    /// is disabled; returns `None` with a warn when the chain is L2 or the
    /// build lacks rayon (or has eip-8025 active).
    pub fn spawn(blockchain: Arc<Blockchain>) -> Option<PrewarmHandle> {
        let opts = blockchain.options.mempool_prewarm.clone();
        if !opts.enabled {
            return None;
        }
        if !matches!(blockchain.options.r#type, BlockchainType::L1) {
            warn!("mempool prewarm is L1-only; disabled");
            return None;
        }
        #[cfg(any(not(feature = "rayon"), feature = "eip-8025"))]
        {
            warn!(
                "mempool prewarm requires the rayon feature and is unavailable on eip-8025 builds; disabled"
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
                .ok()?;
            let (sender, receiver) = mpsc::channel::<PrewarmRequest>();
            let last_warmed: Arc<Mutex<Option<FxHashMap<H256, u64>>>> = Arc::new(Mutex::new(None));
            let last_warmed_worker = last_warmed.clone();
            let slot_duration_secs = opts.slot_duration_secs;
            std::thread::Builder::new()
                .name("mempool_prewarmer".to_string())
                .spawn(move || {
                    while let Ok(mut req) = receiver.recv() {
                        // Drain to the newest request; stale ones are already cancelled.
                        while let Ok(next) = receiver.try_recv() {
                            req = next;
                        }
                        run_pass(&blockchain, &pool, req, &last_warmed_worker, &opts);
                    }
                })
                .ok()?;
            Some(PrewarmHandle {
                sender,
                current_cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
                last_warmed,
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
    last_warmed: &Mutex<Option<FxHashMap<H256, u64>>>,
    opts: &MempoolPrewarmOptions,
) {
    use crate::mempool::PendingTxFilter;
    use crate::vm::StoreVmDatabase;
    use ethrex_common::types::{
        ELASTICITY_MULTIPLIER, calc_excess_blob_gas, calculate_base_fee_per_gas,
    };
    use ethrex_levm::vm::VMType;
    use ethrex_vm::backends::CachingDatabase;
    use ethrex_vm::backends::levm::LEVM;
    use std::time::Instant;
    use tracing::debug;

    if req.cancel.load(Ordering::Relaxed) || unix_now() >= req.deadline_unix {
        debug!(
            "prewarm pass for child of block {} skipped: stale at start",
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

    // Snapshot the mempool. blob_fee: None on purpose — blob txs are warmed
    // like any other tx (the sidecar is not state), no fee gate needed.
    let filter = PendingTxFilter {
        base_fee,
        ..Default::default()
    };
    let txs_by_sender = match blockchain.mempool.filter_transactions(&filter) {
        Ok(txs_by_sender) => txs_by_sender,
        Err(e) => {
            warn!("prewarm pass skipped: mempool snapshot failed: {e}");
            return;
        }
    };
    let gas_budget = opts.gas_budget_multiplier.saturating_mul(parent.gas_limit);
    let warm_set = select_warm_set(txs_by_sender, base_fee, gas_budget);
    if warm_set.is_empty() {
        debug!("prewarm pass: empty warm set, skipping");
        return;
    }

    let warmed_map: FxHashMap<H256, u64> = warm_set
        .iter()
        .map(|(tx, _)| (tx.hash(&NativeCrypto), tx.gas_limit()))
        .collect();
    if let Ok(mut w) = last_warmed.lock() {
        *w = Some(warmed_map);
    }

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
            warn!("prewarm pass skipped: state db unavailable: {e}");
            return;
        }
    };
    let inner: Arc<dyn ethrex_vm::backends::LevmDatabase> =
        Arc::new(Box::new(vm_db) as ethrex_vm::DynVmDatabase);
    let caching: Arc<dyn ethrex_vm::backends::LevmDatabase> = Arc::new(CachingDatabase::new(
        inner,
        blockchain.options.precompile_cache_enabled,
    ));

    let cancel = req.cancel.clone();
    let deadline = req.deadline_unix;
    let should_stop = move || cancel.load(Ordering::Relaxed) || unix_now() >= deadline;

    // `warm_txs` takes borrowed transactions; bridge the owned warm set.
    let view: Vec<(&Transaction, Address)> = warm_set.iter().map(|(tx, s)| (tx, *s)).collect();

    let result = pool.install(|| {
        LEVM::warm_txs(
            &view,
            &header,
            caching,
            VMType::L1,
            &NativeCrypto,
            &should_stop,
        )
    });

    let stop_reason = if req.cancel.load(Ordering::Relaxed) {
        "cancelled"
    } else if unix_now() >= deadline {
        "deadline"
    } else {
        "completed"
    };
    let total_gas: u64 = warm_set.iter().map(|(tx, _)| tx.gas_limit()).sum();
    info!(
        "prewarm pass for block {}: {} txs, {} gas, {:?}, stop={}, err={}",
        header.number,
        warm_set.len(),
        total_gas,
        start.elapsed(),
        stop_reason,
        result.is_err(),
    );
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

    #[test]
    fn overlap_counts_matched_txs_and_gas() {
        use ethrex_common::types::{Block, BlockBody, BlockHeader};
        use ethrex_crypto::NativeCrypto;
        let (_, mtx) = make_tx(0xaa, 0, 100, 50, 21_000);
        let tx = mtx.transaction().clone();
        let mut warmed = FxHashMap::default();
        warmed.insert(tx.hash(&NativeCrypto), tx.gas_limit());
        let block = Block::new(
            BlockHeader::default(),
            BlockBody {
                transactions: vec![tx],
                ommers: vec![],
                withdrawals: None,
            },
        );
        let s = compute_overlap(&warmed, &block);
        assert_eq!(s.matched_txs, 1);
        assert_eq!(s.block_txs, 1);
        assert_eq!(s.matched_gas, 21_000);
        assert_eq!(s.block_gas, 21_000);
    }
}
