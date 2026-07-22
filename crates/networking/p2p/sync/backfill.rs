//! Optional historical chain backfill (block bodies + receipts below the
//! snap-sync pivot), enabled via `--history.chain`.
//!
//! [`run_history_backfill`] is the background task: it reconciles the frontier
//! (`earliest_block_number`), resolves the floor for the configured mode, and
//! reverse-fills bodies + receipts from peers down to that floor, one bounded,
//! validated batch at a time, persisting progress so it resumes across restarts.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ethrex_common::types::{BlockHeader, BlockNumber};
use ethrex_storage::{BackfilledBlock, Store};
use tokio::time::{Duration, sleep};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::peer_handler::{MAX_BLOCK_BODIES_TO_REQUEST, PeerHandler};

use super::{HistoryChain, SyncDiagnostics, SyncError};

/// Resolve the floor block for `--history.chain postmerge`: the merge (Paris)
/// activation block, i.e. the first proof-of-stake block.
///
/// ethrex has no per-network merge-block constant and mainnet merged by TTD
/// (`merge_netsplit_block == None`), so this is a hybrid:
/// 1. use `merge_netsplit_block` when the network configures it (netsplit
///    testnets, and PoS-from-genesis nets that set it to `0`);
/// 2. otherwise bisect the header chain for the first block with
///    `difficulty == 0` — the PoW→PoS boundary. This reuses the same
///    proof-of-stake discriminator the block validator and genesis loader
///    already use, needs no maintained per-network constant table, and works on
///    custom devnets. On mainnet it yields 15_537_394.
///
/// Returns `Ok(None)` when the chain has not merged (head is still PoW), meaning
/// there is no post-merge segment to backfill.
///
/// Precondition: the canonical header chain is present from genesis to head,
/// which holds once snap sync has completed.
pub async fn resolve_postmerge_floor(store: &Store) -> Result<Option<BlockNumber>, SyncError> {
    if let Some(merge_block) = store.get_chain_config().merge_netsplit_block {
        return Ok(Some(merge_block));
    }
    let head = store.get_latest_block_number().await?;
    first_pos_block(head, |n| is_pos_block(store, n))
}

/// Whether canonical block `n` is proof-of-stake, detected by `difficulty == 0`
/// (post-merge blocks carry zero difficulty). A missing header signals a corrupt
/// DB: this is only ever called over the already-synced canonical chain.
fn is_pos_block(store: &Store, n: BlockNumber) -> Result<bool, SyncError> {
    let header = store.get_block_header(n)?.ok_or(SyncError::CorruptDB)?;
    Ok(header.difficulty.is_zero())
}

/// First index in `[0, head]` where `is_pos` holds, assuming `is_pos` is
/// monotonic (false for PoW blocks, then true from the merge block onward).
/// Returns `Ok(None)` when even `head` is still PoW (the chain has not merged).
///
/// Runs in O(log head) evaluations of `is_pos`.
fn first_pos_block<F, E>(head: BlockNumber, mut is_pos: F) -> Result<Option<BlockNumber>, E>
where
    F: FnMut(BlockNumber) -> Result<bool, E>,
{
    if !is_pos(head)? {
        return Ok(None);
    }
    let (mut lo, mut hi) = (0u64, head);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if is_pos(mid)? {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Ok(Some(lo))
}

/// Recompute the true backfill frontier: the lowest block in the head-contiguous
/// run of stored bodies. That is the snap pivot on a snap-synced node (everything
/// below it is headers-only), or genesis on a full-synced node.
///
/// Nodes that snap-synced before this feature left `earliest_block_number` at
/// genesis, which would make backfill think it is already complete. Bodies are
/// present exactly on `[pivot, head]` (plus, possibly, genesis in isolation), so
/// over `[1, head]` the "has a body" predicate is monotonic and we bisect for the
/// pivot.
async fn reconcile_frontier(store: &Store) -> Result<BlockNumber, SyncError> {
    let head = store.get_latest_block_number().await?;
    if head == 0 {
        return Ok(0);
    }
    // A body at block 1 ⇒ full history is present from genesis.
    if store.get_block_body(1).await?.is_some() {
        return Ok(0);
    }
    // No body at head ⇒ not yet synced to the tip; nothing to reconcile against.
    if store.get_block_body(head).await?.is_none() {
        return Ok(head);
    }
    let (mut lo, mut hi) = (1u64, head);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if store.get_block_body(mid).await?.is_some() {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Ok(lo)
}

/// Blocks fetched per backfill batch, bounded by the eth `GetBlockBodies` limit.
const BACKFILL_BATCH_SIZE: u64 = MAX_BLOCK_BODIES_TO_REQUEST as u64;
/// Default pause between successful batches (`--history.backfill-interval-ms`).
/// A short pause yields peers/bandwidth to head-following sync; lowering it
/// speeds up backfill on well-connected nodes, raising it is more polite.
pub const DEFAULT_BACKFILL_BATCH_INTERVAL_MS: u64 = 500;
/// Default number of batches fetched concurrently (`--history.backfill-parallelism`).
/// `1` = one batch at a time (conservative). Higher values fetch that many
/// disjoint ranges from different peers at once, scaling throughput roughly
/// linearly until the peer set / bandwidth is the limit.
pub const DEFAULT_BACKFILL_PARALLELISM: usize = 1;
/// Backoff when a batch makes no progress (no peers, incomplete response) or
/// while initial sync is still running.
const BACKFILL_IDLE_INTERVAL: Duration = Duration::from_secs(10);

/// Configuration for the historical-chain backfill task.
#[derive(Debug, Clone)]
pub struct BackfillConfig {
    pub mode: HistoryChain,
    /// `--history.transactions`: maintain the transaction-lookup index for the
    /// most recent `N` backfilled blocks (`0` = the entire backfilled range).
    pub tx_index_horizon: u64,
    /// `--history.backfill-interval-ms`: pause between successful batches.
    pub batch_interval: Duration,
    /// `--history.backfill-parallelism`: batches fetched concurrently per round.
    pub parallelism: usize,
}

impl Default for BackfillConfig {
    fn default() -> Self {
        Self {
            mode: HistoryChain::Off,
            tx_index_horizon: 0,
            batch_interval: Duration::from_millis(DEFAULT_BACKFILL_BATCH_INTERVAL_MS),
            parallelism: DEFAULT_BACKFILL_PARALLELISM,
        }
    }
}

/// Outcome of a single backfill step, used to pace the loop.
enum BackfillProgress {
    /// A batch was written; the frontier advanced.
    Advanced,
    /// Nothing left to fill (frontier reached the floor, or the chain has not
    /// merged for `postmerge`).
    Complete,
    /// Cannot make progress right now (initial sync running, or no peer/response).
    Waiting,
}

/// Background task that backfills historical block bodies and receipts below the
/// snap-sync pivot, down to the floor implied by `config.mode`.
///
/// It fills in reverse (pivot → floor), one bounded batch at a time, driving the
/// frontier (`earliest_block_number`) downward and persisting it after each
/// batch so the work resumes across restarts. It is best-effort and lower
/// priority than head-following sync: it waits while initial sync runs, sleeps
/// between batches, and never advances the frontier past a hole.
///
/// Runs until the token is cancelled; errors are logged and retried rather than
/// propagated, since this is a non-critical background process.
pub async fn run_history_backfill(
    peers: PeerHandler,
    store: Store,
    config: BackfillConfig,
    snap_enabled: Arc<AtomicBool>,
    cancel_token: CancellationToken,
    diagnostics: Arc<tokio::sync::RwLock<SyncDiagnostics>>,
) {
    if config.mode == HistoryChain::Off {
        return;
    }
    info!(mode = ?config.mode, horizon = config.tx_index_horizon, "Historical chain backfill enabled");

    // One-time frontier reconciliation guard (see `backfill_step`).
    let mut reconciled = false;
    loop {
        if cancel_token.is_cancelled() {
            return;
        }
        let delay = match backfill_step(
            &peers,
            &store,
            &config,
            &snap_enabled,
            &diagnostics,
            &mut reconciled,
        )
        .await
        {
            Ok(BackfillProgress::Advanced) => config.batch_interval,
            Ok(BackfillProgress::Complete | BackfillProgress::Waiting) => BACKFILL_IDLE_INTERVAL,
            Err(e) => {
                warn!("History backfill step failed (will retry): {e}");
                BACKFILL_IDLE_INTERVAL
            }
        };
        tokio::select! {
            _ = sleep(delay) => {}
            _ = cancel_token.cancelled() => return,
        }
    }
}

/// Performs one backfill step: resolve the floor, read the next batch of
/// (already-canonical) headers just below the frontier, fetch and validate their
/// bodies and receipts, persist them, and lower the frontier.
async fn backfill_step(
    peers: &PeerHandler,
    store: &Store,
    config: &BackfillConfig,
    snap_enabled: &AtomicBool,
    diagnostics: &Arc<tokio::sync::RwLock<SyncDiagnostics>>,
    reconciled: &mut bool,
) -> Result<BackfillProgress, SyncError> {
    // Don't compete with initial sync; wait until it has finished.
    if snap_enabled.load(Ordering::Relaxed) {
        return Ok(BackfillProgress::Waiting);
    }

    // One-time correction of `earliest_block_number` for nodes that synced
    // before this feature existed (where it was left at genesis) or otherwise
    // drifted from the true lowest-full-data block. Without this, backfill would
    // see `frontier == 0` and conclude there is nothing to do.
    if !*reconciled {
        let recorded = store.get_earliest_block_number().await?;
        let actual = reconcile_frontier(store).await?;
        if recorded != actual {
            info!(
                recorded,
                actual, "Reconciled backfill frontier to the lowest block with full chain data"
            );
            store.update_earliest_block_number(actual).await?;
        }
        *reconciled = true;
    }

    let floor = match config.mode {
        HistoryChain::Off => return Ok(BackfillProgress::Complete),
        HistoryChain::All => 0,
        HistoryChain::PostMerge => match resolve_postmerge_floor(store).await? {
            Some(floor) => floor,
            // Chain has not merged: there is no post-merge segment to backfill.
            None => return Ok(BackfillProgress::Complete),
        },
    };

    let frontier = store.get_earliest_block_number().await?;
    {
        let mut diag = diagnostics.write().await;
        diag.backfill_mode = Some(format!("{:?}", config.mode));
        diag.backfill_floor = Some(floor);
        diag.backfill_frontier = Some(frontier);
        diag.backfill_complete = frontier <= floor;
    }
    if frontier <= floor {
        return Ok(BackfillProgress::Complete);
    }

    // Dispatch up to `parallelism` disjoint batches stacked below the frontier,
    // fetch them concurrently, then commit the contiguous prefix in order so the
    // on-disk `[earliest, head]` never has a hole (see `plan_backfill_commit`).
    let ranges = plan_backfill_ranges(frontier, floor, BACKFILL_BATCH_SIZE, config.parallelism);
    let head = store.get_latest_block_number().await?;
    let results = futures::future::join_all(
        ranges
            .iter()
            .map(|&(lo, hi)| fetch_range(peers.clone(), store, config, lo, hi, head)),
    )
    .await;

    let filled: Vec<Option<u64>> = results
        .iter()
        .map(|r| r.as_ref().ok().filter(|b| !b.is_empty()).map(|b| b.len() as u64))
        .collect();
    let (commit_count, new_frontier) = plan_backfill_commit(frontier, &ranges, &filled);

    // Write the committed prefix in ascending index (descending block) order,
    // each range lowering the frontier. Surface a hard fetch error only when the
    // round made no progress at all; otherwise the persisted progress stands and
    // the failed lower ranges are simply retried next round.
    for (i, result) in results.into_iter().enumerate() {
        match result {
            Ok(blocks) if i < commit_count => {
                let (_, hi) = ranges[i];
                let batch_earliest = hi - blocks.len() as u64;
                store.add_backfilled_blocks(blocks, batch_earliest).await?;
            }
            Err(e) if new_frontier == frontier => return Err(e),
            _ => {}
        }
    }

    {
        let mut diag = diagnostics.write().await;
        diag.backfill_frontier = Some(new_frontier);
        diag.backfill_complete = new_frontier <= floor;
    }

    if new_frontier == frontier {
        Ok(BackfillProgress::Waiting) // nothing fetched this round
    } else if new_frontier <= floor {
        info!(floor, "Historical chain backfill complete");
        Ok(BackfillProgress::Complete)
    } else {
        debug!(new_earliest = new_frontier, floor, "History backfill advanced");
        Ok(BackfillProgress::Advanced)
    }
}

/// Builds up to `parallelism` disjoint batches of at most `batch` blocks each,
/// stacked below `frontier` down to `floor`. Ranges are `[lo, hi)` in descending
/// order, contiguous (`ranges[i].hi == ranges[i-1].lo`), with `ranges[0].hi ==
/// frontier`; the last is clamped so no range dips below `floor`.
fn plan_backfill_ranges(
    frontier: u64,
    floor: u64,
    batch: u64,
    parallelism: usize,
) -> Vec<(u64, u64)> {
    let parallelism = parallelism.max(1);
    let mut ranges = Vec::with_capacity(parallelism);
    let mut hi = frontier;
    while ranges.len() < parallelism && hi > floor {
        let lo = hi.saturating_sub(batch).max(floor);
        ranges.push((lo, hi));
        hi = lo;
    }
    ranges
}

/// Decides how many of the dispatched `ranges` to commit this round and the
/// resulting frontier, given how many blocks each range actually returned
/// (`filled[i]`: `Some(n)` fetched, `None` = failed/empty).
///
/// Commits the maximal contiguous prefix — starting at the frontier — of ranges
/// that came back **full**, plus the first partial range if any, then stops.
/// This guarantees the committed blocks are contiguous with the frontier, so the
/// persisted `[earliest, head]` never gains a hole (which would break the
/// crash-resume/reconcile bisect). Returns `(commit_count, new_frontier)`.
fn plan_backfill_commit(
    frontier: u64,
    ranges: &[(u64, u64)],
    filled: &[Option<u64>],
) -> (usize, u64) {
    let mut new_frontier = frontier;
    let mut commit_count = 0;
    for (i, &(lo, hi)) in ranges.iter().enumerate() {
        // Contiguity: this range must sit directly below what we've committed.
        if hi != new_frontier {
            break;
        }
        let Some(n) = filled.get(i).copied().flatten() else {
            break; // range failed or returned nothing
        };
        if n == 0 {
            break;
        }
        new_frontier = hi - n;
        commit_count = i + 1;
        if n < hi - lo {
            break; // truncated response: hole below, stop after committing this one
        }
    }
    (commit_count, new_frontier)
}

/// Fetches bodies and receipts for the canonical range `[lo, hi)` concurrently
/// from peers and returns the contiguous run — from `hi - 1` downward — for
/// which BOTH were obtained and validated against the stored headers. An empty
/// vec means no peer served the range this round (a soft miss the caller retries
/// later); errors are reserved for hard failures (DB corruption, peer actor).
async fn fetch_range(
    peers: PeerHandler,
    store: &Store,
    config: &BackfillConfig,
    lo: u64,
    hi: u64,
    head: BlockNumber,
) -> Result<Vec<BackfilledBlock>, SyncError> {
    // Read headers top-down (highest first): peers return bodies/receipts in
    // request order, so a truncated response still yields a run contiguous with
    // the top of the range.
    let mut headers: Vec<BlockHeader> = Vec::with_capacity((hi - lo) as usize);
    for number in (lo..hi).rev() {
        headers.push(store.get_block_header(number)?.ok_or(SyncError::CorruptDB)?);
    }

    // Fetch bodies and receipts concurrently (each from its own peer) to halve
    // per-range latency. Both are validated against the headers inside the
    // request (block-body validation; receipts root recomputed from logs, which
    // reconstructs eth/69's omitted bloom). PeerHandler is a cheap Arc-backed
    // handle, so cloning it to hold two requests at once is fine.
    let (mut peers_bodies, mut peers_receipts) = (peers.clone(), peers);
    let (bodies_res, receipts_res) = tokio::join!(
        peers_bodies.request_block_bodies(&headers),
        peers_receipts.request_receipts(&headers),
    );
    let (Some(bodies), Some(receipts)) = (bodies_res?, receipts_res?) else {
        return Ok(Vec::new());
    };

    // Only blocks with BOTH a body and receipts count; since both responses are
    // prefixes of the top-down header list, their common prefix is contiguous
    // from the top of the range downward.
    let filled = bodies.len().min(receipts.len());
    if filled == 0 {
        return Ok(Vec::new());
    }

    let horizon = config.tx_index_horizon;
    Ok(headers
        .into_iter()
        .zip(bodies)
        .zip(receipts)
        .take(filled)
        .map(|((header, body), block_receipts)| {
            let index_transactions = horizon == 0 || head.saturating_sub(header.number) < horizon;
            BackfilledBlock {
                header,
                body,
                receipts: block_receipts,
                index_transactions,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mainnet-shaped: the merge sits deep inside a long chain. Bisect must land
    // on the exact boundary (mainnet Paris block = 15_537_394).
    #[test]
    fn finds_merge_block_mid_chain() {
        let merge = 15_537_394u64;
        let found = first_pos_block::<_, ()>(25_000_000, |n| Ok(n >= merge)).unwrap();
        assert_eq!(found, Some(merge));
    }

    // PoS from genesis (difficulty 0 throughout): floor is genesis, so
    // `postmerge` collapses to `all`.
    #[test]
    fn pos_from_genesis_returns_zero() {
        let found = first_pos_block::<_, ()>(1_000, |_| Ok(true)).unwrap();
        assert_eq!(found, Some(0));
    }

    // Merge exactly at head.
    #[test]
    fn merge_at_head_is_found() {
        let head = 1_000u64;
        let found = first_pos_block::<_, ()>(head, |n| Ok(n >= head)).unwrap();
        assert_eq!(found, Some(head));
    }

    // Never-merged PoW chain: no post-merge segment exists.
    #[test]
    fn never_merged_chain_returns_none() {
        let found = first_pos_block::<_, ()>(1_000, |_| Ok(false)).unwrap();
        assert_eq!(found, None);
    }

    // The whole point of A over a constant table is that this stays cheap: a
    // handful of header reads at backfill start, not a linear scan.
    #[test]
    fn bisect_is_logarithmic() {
        let merge = 15_537_394u64;
        let mut reads = 0u32;
        let found = first_pos_block::<_, ()>(25_000_000, |n| {
            reads += 1;
            Ok(n >= merge)
        })
        .unwrap();
        assert_eq!(found, Some(merge));
        // log2(25e6) ≈ 24.6; allow headroom incl. the initial head probe.
        assert!(
            reads <= 30,
            "bisect made {reads} reads, expected ~log2(head)"
        );
    }

    // Errors from the predicate propagate (e.g. a missing header → corrupt DB).
    #[test]
    fn predicate_error_propagates() {
        let result = first_pos_block::<_, &str>(1_000, |_| Err("boom"));
        assert_eq!(result, Err("boom"));
    }

    // --- reconcile_frontier: recompute the true frontier on a real store ---

    use ethrex_common::types::BlockBody;
    use ethrex_storage::EngineType;

    /// Build an in-memory store with canonical headers `0..=head` and block
    /// bodies present only for `[pivot, head]` — the shape of a snap-synced node
    /// (headers-only below the pivot).
    async fn store_with_bodies_from(pivot: u64, head: u64) -> Store {
        let store = Store::new("", EngineType::InMemory).expect("in-memory store");
        let headers: Vec<BlockHeader> = (0..=head)
            .map(|number| BlockHeader {
                number,
                ..Default::default()
            })
            .collect();
        store.add_block_headers(headers.clone()).await.unwrap();
        let canonical: Vec<_> = headers.iter().map(|h| (h.number, h.hash())).collect();
        store
            .forkchoice_update(canonical, head, headers[head as usize].hash(), None, None)
            .await
            .unwrap();
        for number in pivot..=head {
            store
                .add_block_body(
                    headers[number as usize].hash(),
                    BlockBody {
                        transactions: vec![],
                        ommers: vec![],
                        withdrawals: Some(vec![]),
                    },
                )
                .await
                .unwrap();
        }
        store
    }

    /// On a snap-synced node (bodies only from the pivot up), the frontier is the
    /// pivot — even though `earliest_block_number` was left at genesis.
    #[tokio::test]
    async fn reconcile_frontier_finds_the_pivot_on_a_snap_node() {
        let store = store_with_bodies_from(50, 100).await;
        assert_eq!(reconcile_frontier(&store).await.unwrap(), 50);
    }

    /// On a full-synced node (bodies from block 1), the frontier is genesis.
    #[tokio::test]
    async fn reconcile_frontier_is_genesis_on_a_full_node() {
        let store = store_with_bodies_from(1, 100).await;
        assert_eq!(reconcile_frontier(&store).await.unwrap(), 0);
    }

    // ---- plan_backfill_ranges ----

    #[test]
    fn ranges_are_contiguous_and_descending_from_the_frontier() {
        let r = plan_backfill_ranges(1000, 0, 128, 4);
        assert_eq!(r, vec![(872, 1000), (744, 872), (616, 744), (488, 616)]);
        // contiguous: each hi == previous lo, top == frontier
        assert_eq!(r[0].1, 1000);
        for w in r.windows(2) {
            assert_eq!(w[0].0, w[1].1);
        }
    }

    #[test]
    fn ranges_clamp_to_the_floor_and_never_dip_below_it() {
        // Only 100 blocks above the floor but room for 4×128: one clamped range.
        let r = plan_backfill_ranges(200, 100, 128, 4);
        assert_eq!(r, vec![(100, 200)]);
        assert!(r.iter().all(|&(lo, _)| lo >= 100));
    }

    #[test]
    fn ranges_parallelism_one_is_a_single_batch() {
        assert_eq!(plan_backfill_ranges(1000, 0, 128, 1), vec![(872, 1000)]);
        // parallelism 0 is treated as 1 rather than yielding no work.
        assert_eq!(plan_backfill_ranges(1000, 0, 128, 0), vec![(872, 1000)]);
    }

    #[test]
    fn ranges_empty_when_frontier_at_floor() {
        assert!(plan_backfill_ranges(100, 100, 128, 4).is_empty());
    }

    // ---- plan_backfill_commit ----

    // Helper: full-size ranges stacked below `frontier`.
    fn ranges(frontier: u64, n: usize) -> Vec<(u64, u64)> {
        plan_backfill_ranges(frontier, 0, 128, n)
    }

    #[test]
    fn commit_all_full_ranges_advances_by_the_whole_wave() {
        let r = ranges(1000, 3);
        let (count, frontier) = plan_backfill_commit(1000, &r, &[Some(128), Some(128), Some(128)]);
        assert_eq!((count, frontier), (3, 1000 - 3 * 128));
    }

    #[test]
    fn commit_stops_at_a_partial_range_but_keeps_it() {
        // Second range truncated: commit range 0 fully + range 1 partially, stop.
        let r = ranges(1000, 3);
        let (count, frontier) = plan_backfill_commit(1000, &r, &[Some(128), Some(50), Some(128)]);
        assert_eq!(count, 2);
        assert_eq!(frontier, 1000 - 128 - 50);
    }

    #[test]
    fn commit_stops_before_an_empty_or_failed_range() {
        let r = ranges(1000, 3);
        // Range 1 returned nothing: only range 0 commits, range 2 is not reached
        // (committing it would leave a hole at range 1).
        assert_eq!(
            plan_backfill_commit(1000, &r, &[Some(128), Some(0), Some(128)]),
            (1, 872)
        );
        // A failed (None) range behaves the same as empty.
        assert_eq!(
            plan_backfill_commit(1000, &r, &[Some(128), None, Some(128)]),
            (1, 872)
        );
    }

    #[test]
    fn commit_nothing_when_the_first_range_is_empty() {
        let r = ranges(1000, 3);
        assert_eq!(plan_backfill_commit(1000, &r, &[None, Some(128)]), (0, 1000));
        assert_eq!(
            plan_backfill_commit(1000, &r, &[Some(0), Some(128)]),
            (0, 1000)
        );
    }

    #[test]
    fn commit_reaches_the_floor() {
        // frontier 200, floor 100: a single clamped 100-block range, filled full.
        let r = plan_backfill_ranges(200, 100, 128, 4);
        assert_eq!(r, vec![(100, 200)]);
        assert_eq!(plan_backfill_commit(200, &r, &[Some(100)]), (1, 100));
    }
}
