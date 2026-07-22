//! Optional historical chain backfill (block bodies + receipts below the
//! snap-sync pivot), enabled via `--history.chain`.
//!
//! [`run_history_backfill`] is the background task: it reconciles the frontier
//! (`earliest_block_number`), resolves the floor for the configured mode, and
//! reverse-fills bodies + receipts from peers down to that floor, one bounded,
//! validated batch at a time, persisting progress so it resumes across restarts.

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ethrex_common::types::{BlockHeader, BlockNumber};
use ethrex_storage::{BackfilledBlock, Store};
use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
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

/// The result of one range fetch: `(lo, hi, blocks-or-error)`.
type FetchOutcome = (u64, u64, Result<Vec<BackfilledBlock>, SyncError>);

/// Background task that backfills historical block bodies and receipts below the
/// snap-sync pivot, down to the floor implied by `config.mode`.
///
/// It fills in reverse (pivot → floor), driving the frontier
/// (`earliest_block_number`) downward and persisting it after each batch so the
/// work resumes across restarts. It is best-effort and lower priority than
/// head-following sync: it waits while initial sync runs and yields between
/// batches. With `--history.backfill-parallelism > 1` it keeps several fetches in
/// flight at once (see [`run_pipeline`]).
///
/// Runs until the token is cancelled; a fatal error stops the task (logged) but
/// never crashes the node, and progress already persisted is safe to resume.
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
    info!(
        mode = ?config.mode,
        horizon = config.tx_index_horizon,
        parallelism = config.parallelism,
        "Historical chain backfill enabled"
    );

    // Don't compete with initial sync; wait until it has finished.
    while snap_enabled.load(Ordering::Relaxed) {
        tokio::select! {
            _ = sleep(BACKFILL_IDLE_INTERVAL) => {}
            _ = cancel_token.cancelled() => return,
        }
    }

    // One-time correction of `earliest_block_number` for nodes that synced before
    // this feature existed (left at genesis) or otherwise drifted from the true
    // lowest-full-data block. Without this, backfill would see `frontier == 0`
    // and conclude there is nothing to do.
    if let Err(e) = reconcile_once(&store).await {
        warn!("History backfill frontier reconciliation failed: {e}");
        return;
    }

    // Resolve the floor once; it is constant for a given mode/network.
    let floor = match config.mode {
        HistoryChain::Off => return,
        HistoryChain::All => 0,
        HistoryChain::PostMerge => match resolve_postmerge_floor(&store).await {
            Ok(Some(floor)) => floor,
            Ok(None) => return, // chain has not merged: nothing to backfill
            Err(e) => {
                warn!("History backfill could not resolve the post-merge floor: {e}");
                return;
            }
        },
    };

    if let Err(e) = run_pipeline(
        &peers,
        &store,
        &config,
        floor,
        &snap_enabled,
        &cancel_token,
        &diagnostics,
    )
    .await
    {
        warn!("History backfill stopped on a fatal error: {e}");
    }
}

/// One-time correction of the frontier to the lowest block with full chain data.
async fn reconcile_once(store: &Store) -> Result<(), SyncError> {
    let recorded = store.get_earliest_block_number().await?;
    let actual = reconcile_frontier(store).await?;
    if recorded != actual {
        info!(
            recorded,
            actual, "Reconciled backfill frontier to the lowest block with full chain data"
        );
        store.update_earliest_block_number(actual).await?;
    }
    Ok(())
}

/// The batch `[lo, hi)` immediately below `hi`, clamped to `floor`. `None` once
/// `hi` has reached the floor.
fn range_below(hi: u64, floor: u64, batch: u64) -> Option<(u64, u64)> {
    (hi > floor).then(|| (hi.saturating_sub(batch).max(floor), hi))
}

/// Reverse-fills bodies + receipts from the frontier down to `floor`, keeping up
/// to `config.parallelism` fetches in flight and committing each batch the moment
/// it becomes contiguous with the frontier — so a slow peer never stalls the
/// others (unlike a per-round barrier).
///
/// Only **full** batches are committed; short/empty/failed fetches are retried,
/// which keeps the frontier on a fixed grid and the persisted `[earliest, head]`
/// hole-free (preserving the crash-resume/reconcile guarantees). At most
/// `parallelism` ranges are outstanding (in flight + buffered), bounding memory.
async fn run_pipeline(
    peers: &PeerHandler,
    store: &Store,
    config: &BackfillConfig,
    floor: u64,
    snap_enabled: &AtomicBool,
    cancel_token: &CancellationToken,
    diagnostics: &Arc<tokio::sync::RwLock<SyncDiagnostics>>,
) -> Result<(), SyncError> {
    let parallelism = config.parallelism.max(1);
    let head = store.get_latest_block_number().await?;
    let mut frontier = store.get_earliest_block_number().await?;

    {
        let mut diag = diagnostics.write().await;
        diag.backfill_mode = Some(format!("{:?}", config.mode));
        diag.backfill_floor = Some(floor);
        diag.backfill_frontier = Some(frontier);
        diag.backfill_complete = frontier <= floor;
    }

    // Builds a boxed fetch future so initial dispatch and retry push the same
    // (nameable) type into the `FuturesUnordered`.
    let make_fetch = |lo: u64, hi: u64| -> BoxFuture<'static, FetchOutcome> {
        let (peers, store, config) = (peers.clone(), store.clone(), config.clone());
        Box::pin(async move {
            let r = fetch_range(peers, &store, &config, lo, hi, head).await;
            (lo, hi, r)
        })
    };

    // `dispatch_lo` = top of the next range to dispatch (ranges march down from
    // the frontier). `buffer` holds full batches that arrived before their turn
    // at the frontier, keyed by `hi`.
    let mut dispatch_lo = frontier;
    let mut in_flight: FuturesUnordered<BoxFuture<'static, FetchOutcome>> =
        FuturesUnordered::new();
    let mut buffer: HashMap<u64, Vec<BackfilledBlock>> = HashMap::new();
    let mut idle_streak = 0usize;

    loop {
        if cancel_token.is_cancelled() {
            return Ok(());
        }
        if frontier <= floor {
            info!(floor, "Historical chain backfill complete");
            return Ok(());
        }
        // Yield entirely to initial sync if it somehow restarts.
        if snap_enabled.load(Ordering::Relaxed) {
            tokio::select! {
                _ = sleep(BACKFILL_IDLE_INTERVAL) => {}
                _ = cancel_token.cancelled() => return Ok(()),
            }
            continue;
        }

        // Keep up to `parallelism` ranges outstanding (in flight + buffered).
        while in_flight.len() + buffer.len() < parallelism {
            let Some((lo, hi)) = range_below(dispatch_lo, floor, BACKFILL_BATCH_SIZE) else {
                break; // reached the floor; drain what is already outstanding
            };
            dispatch_lo = lo;
            in_flight.push(make_fetch(lo, hi));
        }

        let Some((lo, hi, res)) = in_flight.next().await else {
            // Nothing outstanding and nothing left to dispatch: we are done.
            info!(floor, "Historical chain backfill complete");
            return Ok(());
        };

        let mut committed = false;
        match res {
            // Full batch: hold it for in-order commit.
            Ok(blocks) if blocks.len() as u64 == hi - lo => {
                buffer.insert(hi, blocks);
            }
            // Frontier-adjacent short batch: commit the partial to guarantee
            // progress even if no peer fully serves this range, then realign —
            // the buffered lower ranges are no longer contiguous with the new
            // (off-grid) frontier, so drop them and re-dispatch from here.
            Ok(blocks) if hi == frontier && !blocks.is_empty() => {
                let batch_earliest = hi - blocks.len() as u64;
                store.add_backfilled_blocks(blocks, batch_earliest).await?;
                frontier = batch_earliest;
                buffer.clear();
                in_flight.clear();
                dispatch_lo = frontier;
                committed = true;
            }
            // Short (not yet at the frontier) or empty: retry the range.
            Ok(_) => {
                in_flight.push(make_fetch(lo, hi));
                idle_streak += 1;
            }
            // Hard error: retry the range (peer actors recover; transient DB).
            Err(e) => {
                warn!("History backfill fetch error for [{lo}, {hi}) (retrying): {e}");
                in_flight.push(make_fetch(lo, hi));
                idle_streak += 1;
            }
        }

        // Commit every buffered batch now contiguous with the frontier.
        while let Some(blocks) = buffer.remove(&frontier) {
            let batch_earliest = frontier - blocks.len() as u64;
            store.add_backfilled_blocks(blocks, batch_earliest).await?;
            frontier = batch_earliest;
            committed = true;
        }
        if committed {
            idle_streak = 0;
            {
                let mut diag = diagnostics.write().await;
                diag.backfill_frontier = Some(frontier);
                diag.backfill_complete = frontier <= floor;
            }
            debug!(frontier, floor, "History backfill advanced");
            // Politeness pause so backfill yields to head-following sync.
            tokio::select! {
                _ = sleep(config.batch_interval) => {}
                _ = cancel_token.cancelled() => return Ok(()),
            }
        }

        // Back off when the whole outstanding window churned without progress
        // (e.g. no peer is serving history right now), so we don't spin.
        if idle_streak >= parallelism {
            idle_streak = 0;
            tokio::select! {
                _ = sleep(BACKFILL_IDLE_INTERVAL) => {}
                _ = cancel_token.cancelled() => return Ok(()),
            }
        }
    }
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

    // ---- range_below ----

    #[test]
    fn range_below_is_a_full_batch_when_room_allows() {
        // A full 128-block batch directly below `hi`.
        assert_eq!(range_below(1000, 0, 128), Some((872, 1000)));
    }

    #[test]
    fn range_below_clamps_to_the_floor() {
        // Fewer than `batch` blocks left above the floor: clamp `lo` to the floor.
        assert_eq!(range_below(200, 100, 128), Some((100, 200)));
        assert!(range_below(200, 100, 128).unwrap().0 >= 100);
    }

    #[test]
    fn range_below_is_none_at_or_below_the_floor() {
        assert_eq!(range_below(100, 100, 128), None);
        assert_eq!(range_below(50, 100, 128), None);
    }

    #[test]
    fn range_below_marches_down_on_a_fixed_grid() {
        // Chaining `range_below` from the frontier yields contiguous, descending
        // batches — the grid the pipeline dispatches and commits on.
        let (floor, batch) = (0, 128);
        let mut hi = 1000;
        let mut seen = vec![];
        while let Some((lo, got_hi)) = range_below(hi, floor, batch) {
            assert_eq!(got_hi, hi); // each batch's top is the previous bottom
            seen.push((lo, got_hi));
            hi = lo;
        }
        assert_eq!(seen[0], (872, 1000));
        for w in seen.windows(2) {
            assert_eq!(w[0].0, w[1].1); // contiguous
        }
        assert_eq!(hi, floor); // marched exactly to the floor
    }
}
