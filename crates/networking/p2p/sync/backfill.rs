//! Optional historical chain backfill (block bodies + receipts below the
//! snap-sync pivot), enabled via `--history.chain`.
//!
//! [`run_history_backfill`] is the background task: it reconciles the frontier
//! (`earliest_block_number`), resolves the floor for the configured mode, and
//! reverse-fills bodies + receipts from peers down to that floor, one bounded,
//! validated batch at a time, persisting progress so it resumes across restarts.

use std::sync::Arc;

use ethrex_blockchain::Blockchain;
use ethrex_common::types::{BlockHeader, BlockNumber};
use ethrex_crypto::NativeCrypto;
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
///    `difficulty == 0` — the PoW→PoS boundary. EIP-3675 pins `difficulty` to the
///    constant `0` from `TRANSITION_BLOCK` onward and makes a non-zero value a
///    validity failure, so on any TTD-merged chain the predicate is monotonic *by
///    consensus* and the bisect precondition holds by rule, not by convention. It
///    also needs no maintained per-network constant table and works on custom
///    devnets. On mainnet it yields 15_537_394.
///
/// Returns `Ok(None)` when the chain has not merged (head is still PoW), meaning
/// there is no post-merge segment to backfill.
///
/// Precondition: the canonical header chain is present from genesis to head,
/// which holds once snap sync has completed.
async fn resolve_postmerge_floor(store: &Store) -> Result<Option<BlockNumber>, SyncError> {
    if let Some(merge_block) = store.get_chain_config().merge_netsplit_block {
        return Ok(Some(merge_block));
    }
    let head = store.get_latest_block_number()?;
    // A block is proof-of-stake iff its difficulty is zero. A missing header signals
    // a corrupt DB: this only ever walks the already-synced canonical chain.
    first_pos_block(head, |n| {
        let header = store.get_block_header(n)?.ok_or(SyncError::CorruptDB)?;
        Ok(header.difficulty.is_zero())
    })
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

/// Resolve the lowest block backfill should fill down to for `mode`.
///
/// The result is clamped at the Byzantium fork block. Before Byzantium
/// (EIP-658), a receipt's first field is a 32-byte post-state root rather than a
/// status flag, which [`ethrex_common::types::Receipt`] has no representation for
/// and cannot decode — so pre-Byzantium receipts can never be fetched, and
/// descending below that block would only fail every request and penalize peers
/// that answered correctly.
///
/// `Ok(None)` means there is nothing to backfill right now: the feature is off,
/// or `PostMerge` was requested on a chain that has not merged yet.
async fn resolve_floor(
    store: &Store,
    mode: &HistoryChain,
) -> Result<Option<BlockNumber>, SyncError> {
    // A chain with no Byzantium block configured is post-Byzantium from genesis
    // (e.g. PoS-from-genesis devnets), so genesis is a valid floor there.
    let byzantium = store.get_chain_config().byzantium_block.unwrap_or(0);
    let requested = match mode {
        HistoryChain::Off => return Ok(None),
        HistoryChain::All => byzantium,
        HistoryChain::Block(floor) => *floor,
        HistoryChain::PostMerge => match resolve_postmerge_floor(store).await? {
            Some(floor) => floor,
            None => return Ok(None),
        },
    };
    if requested < byzantium {
        warn!(
            requested,
            byzantium,
            "Requested history floor is below the Byzantium fork; clamping to it. \
             Pre-Byzantium receipts use the pre-EIP-658 post-state-root format, \
             which ethrex cannot represent."
        );
        return Ok(Some(byzantium));
    }
    Ok(Some(requested))
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
///
/// Also used by snap sync to record the true frontier at the end of each cycle.
pub(crate) async fn reconcile_frontier(store: &Store) -> Result<BlockNumber, SyncError> {
    let head = store.get_latest_block_number()?;
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

/// One-time correction of the stored history bounds for nodes that synced or
/// backfilled before they were tracked: `earliest_block_number` left at genesis
/// by snap sync, and a missing transaction index floor below a
/// `--history.transactions` horizon, which may no longer be configured.
async fn reconcile_history_bounds(store: &Store) -> Result<(), SyncError> {
    let recorded = store.get_earliest_block_number().await?;
    let actual = reconcile_frontier(store).await?;
    if recorded != actual {
        info!(
            recorded,
            actual, "Reconciled backfill frontier to the lowest block with full chain data"
        );
        store.update_earliest_block_number(actual).await?;
    }
    // Marked done even when nothing is recorded, so a fully indexed history is
    // not searched again on every start. Marked last, once `earliest` is correct
    // too: `eth_capabilities` reports no history from an unmarked database, and a
    // stop between the two writes leaves it unmarked, so the next start runs this again.
    if !store.history_bounds_reconciled().await? {
        let mut found = None;
        if store.get_tx_index_floor().await?.is_none() {
            info!("Searching the stored history for blocks an earlier backfill left unindexed");
            let floor = find_tx_index_floor(store, TX_INDEX_WINDOW).await?;
            if floor > actual {
                info!(
                    floor,
                    "Recorded the transaction index floor of an earlier backfill"
                );
                found = Some(floor);
            } else {
                info!("Every stored block is in the transaction index");
            }
        }
        store.mark_history_bounds_reconciled(found).await?;
    }
    Ok(())
}

/// Blocks in each window of [`find_tx_index_floor`], which is also the shortest
/// unindexed stretch it is sure to find. A window that is indexed costs two block
/// checks when its first and last blocks have transactions, where an exact search
/// would check every block.
const TX_INDEX_WINDOW: u64 = 1024;

/// Lowest block from which every stored block's transactions are indexed.
///
/// Backfill leaves out blocks more than the horizon below the head of its run,
/// and each run starts below the last with a later head, so under a horizon that
/// never grew the indexed blocks form one run ending at the head. A horizon raised
/// or removed between runs indexes blocks below ones an earlier run left out: the
/// unindexed blocks then lie in several stretches, and the floor is right above
/// the highest.
///
/// The history is walked from the head down in windows of `window_size` blocks.
/// A window whose lowest and highest blocks with transactions are both indexed is
/// taken as indexed throughout. The first window that is not is read block by
/// block, and the floor is right above its highest unindexed block.
///
/// An unindexed stretch of at least `window_size` blocks is always found: it
/// holds the first block of a window, so it holds the lowest or the highest block
/// with transactions of some window. A shorter stretch that lies between two
/// indexed blocks of one window is missed, and the floor then comes out below it.
/// Ruling that out would take reading every block of the history.
///
/// This completes at most once per database, and never runs on a new one, which
/// is created already marked as reconciled. Empty blocks are read until one with
/// transactions turns up, from each end of a window, so the whole search reads
/// about one pass over the range in the worst case, when nearly every block is
/// empty.
async fn find_tx_index_floor(store: &Store, window_size: u64) -> Result<BlockNumber, SyncError> {
    let earliest = store.get_earliest_block_number().await?;
    let head = store.get_latest_block_number()?;
    // `earliest` never passes the head. It reads as above it when a forkchoice update
    // lowered the head while the two were being read.
    if earliest > head {
        return Err(SyncError::CanonicalChainMoved(head));
    }
    // A zero size would never move the window down.
    let window_size = window_size.max(1);
    let mut window_end = head.saturating_add(1);
    while window_end > earliest {
        let window_start = window_end.saturating_sub(window_size).max(earliest);
        let window = window_start..window_end;
        let unindexed_end =
            match first_with_transactions_is_unindexed(store, window.clone().rev()).await? {
                // No transactions in the window: nothing to check, and no need to
                // read it again from the other end.
                None => false,
                Some(true) => true,
                Some(false) => {
                    first_with_transactions_is_unindexed(store, window.clone()).await? == Some(true)
                }
            };
        if unindexed_end {
            for number in window.rev() {
                if is_unindexed(store, number).await? == Some(true) {
                    return Ok(number + 1);
                }
            }
        }
        window_end = window_start;
    }
    Ok(earliest)
}

/// Whether the first of `blocks` that has transactions is missing from the
/// transaction index. `None` when none of them has transactions.
async fn first_with_transactions_is_unindexed(
    store: &Store,
    blocks: impl Iterator<Item = BlockNumber>,
) -> Result<Option<bool>, SyncError> {
    for number in blocks {
        let unindexed = is_unindexed(store, number).await?;
        if unindexed.is_some() {
            return Ok(unindexed);
        }
    }
    Ok(None)
}

/// Whether the transactions of the canonical block `number` are missing from the
/// transaction index. `None` for an empty block, which says nothing about the index.
/// A block without a body counts as unindexed, which keeps the floor conservative.
///
/// A recorded floor is never lowered, so a forkchoice update running alongside must
/// not be able to produce one. The index is therefore asked about the block by its
/// hash: a block is indexed when it is stored, and whether it is canonical yet, or
/// still, does not change the answer. A height with no canonical block is one such
/// an update is rewriting, and is an error to retry, not an unindexed block. If the
/// hole stays, the database is damaged and goes on reporting no history.
async fn is_unindexed(store: &Store, number: BlockNumber) -> Result<Option<bool>, SyncError> {
    let Some(hash) = store.get_canonical_block_hash(number).await? else {
        return Err(SyncError::CanonicalChainMoved(number));
    };
    let Some(body) = store.get_block_body_by_hash(hash).await? else {
        return Ok(Some(true));
    };
    let Some(tx) = body.transactions.first() else {
        return Ok(None);
    };
    let indexed = store
        .is_transaction_indexed_in(tx.hash(&NativeCrypto), hash)
        .await?;
    Ok(Some(!indexed))
}

/// With `--history.chain off` no backfill runs, but `eth_capabilities` still
/// reports from the history bounds, so reconcile them once the node is synced.
///
/// A node that is never synced, such as an L1 node with no consensus client, is
/// not following the chain, and a database that needs the reconciliation keeps
/// reporting no history from it.
pub async fn reconcile_history_bounds_when_synced(
    store: Store,
    blockchain: Arc<Blockchain>,
    cancel_token: CancellationToken,
) {
    reconcile_history_bounds_when(store, cancel_token, move || blockchain.is_synced()).await
}

/// For a node that starts no sync manager, which would otherwise never reconcile
/// the history bounds of a database created before they were tracked. It does not
/// wait for the node to be synced: without a sync manager nothing is syncing that
/// could still move the bounds. Such a node never backfills, so the search finds no
/// unindexed block on it. It runs all the same, once and in the background, which
/// keeps one reconciliation for every kind of node.
pub async fn reconcile_history_bounds_at_startup(store: Store, cancel_token: CancellationToken) {
    reconcile_history_bounds_when(store, cancel_token, || true).await
}

/// Reconciles the history bounds as soon as `ready` holds, retrying until it
/// succeeds or the token is cancelled.
async fn reconcile_history_bounds_when(
    store: Store,
    cancel_token: CancellationToken,
    ready: impl Fn() -> bool,
) {
    loop {
        if ready() {
            let result = tokio::select! {
                result = reconcile_history_bounds(&store) => result,
                _ = cancel_token.cancelled() => return,
            };
            match result {
                Ok(()) => return,
                Err(e) => warn!("Reconciling the history bounds failed (will retry): {e}"),
            }
        }
        tokio::select! {
            _ = sleep(BACKFILL_IDLE_INTERVAL) => {}
            _ = cancel_token.cancelled() => return,
        }
    }
}

/// Blocks fetched per backfill batch.
///
/// Receipts, not bodies, are the binding constraint: a block's receipts are much
/// larger on the wire than its body (each receipt carries a 256-byte bloom before
/// eth/69), so a full `MAX_BLOCK_BODIES_TO_REQUEST` batch of mainnet receipts runs
/// past EIP-7975's 10 MiB soft response limit and comes back truncated — paying a
/// large request to receive a handful of blocks. This is sized to stay under that
/// limit for recent mainnet blocks.
const BACKFILL_BATCH_SIZE: u64 = 64;
const _: () = assert!(
    BACKFILL_BATCH_SIZE <= MAX_BLOCK_BODIES_TO_REQUEST as u64,
    "batch must not exceed the eth GetBlockBodies limit"
);
/// Pause between successful batches so backfill yields peers/bandwidth to
/// head-following sync rather than saturating them.
const BACKFILL_BATCH_INTERVAL: Duration = Duration::from_millis(500);
/// Consecutive non-advancing steps before backfill reports itself stalled. At
/// `BACKFILL_IDLE_INTERVAL` per step this is a couple of minutes of no progress,
/// long enough not to fire on a transient peer gap.
const BACKFILL_STALL_STEPS: u32 = 12;
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
}

/// Outcome of a single backfill step, used to pace the loop.
enum BackfillProgress {
    /// A batch was written; the frontier advanced.
    Advanced,
    /// The frontier reached the floor. Nothing will ever be left to fill, so the
    /// task can stop rather than idle.
    Complete,
    /// Cannot make progress right now, but the situation can change: initial sync
    /// is running, no peer answered, or `postmerge` was requested on a chain that
    /// has not merged yet.
    Waiting,
}

/// Values resolved once per run, after initial sync has finished, rather than on
/// every batch.
struct BackfillPlan {
    /// Lowest block to fill down to. Fixed for the run: the merge and Byzantium
    /// blocks don't move, and re-deriving the merge block costs a header bisect.
    floor: BlockNumber,
    /// Chain head when the run started, used as the fixed reference for the
    /// `--history.transactions` window. Re-reading the live head each batch would
    /// let the cutoff drift by tens of thousands of blocks over a run that takes
    /// weeks, so the indexed range would not correspond to any single head.
    head: BlockNumber,
    /// `Debug` rendering of the mode for diagnostics; never changes.
    mode_label: String,
    /// Whether the next batch is the first of this run. The first batch includes
    /// the frontier block itself, to repair a snap pivot that has a body but no
    /// receipts; later batches stop below the frontier.
    first_batch: bool,
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
    mut peers: PeerHandler,
    store: Store,
    blockchain: Arc<Blockchain>,
    config: BackfillConfig,
    cancel_token: CancellationToken,
    diagnostics: Arc<tokio::sync::RwLock<SyncDiagnostics>>,
) {
    if config.mode == HistoryChain::Off {
        return;
    }
    info!(mode = ?config.mode, horizon = config.tx_index_horizon, "Historical chain backfill enabled");

    // One-time frontier reconciliation guard (see `backfill_step`).
    let mut reconciled = false;
    // Resolved on the first step that runs after initial sync (see `BackfillPlan`).
    let mut plan: Option<BackfillPlan> = None;
    // Consecutive steps that made no progress, used to tell a genuine stall (no peer
    // serves the range we need) from a healthy idle. Without this a stall is silent:
    // the task just sleeps forever and only a flatlining frontier gauge shows it.
    let mut idle_steps: u32 = 0;
    loop {
        if cancel_token.is_cancelled() {
            return;
        }
        let delay = match backfill_step(
            &mut peers,
            &store,
            &blockchain,
            &config,
            &diagnostics,
            &mut reconciled,
            &mut plan,
        )
        .await
        {
            Ok(BackfillProgress::Advanced) => {
                if idle_steps >= BACKFILL_STALL_STEPS {
                    info!("History backfill resumed");
                }
                idle_steps = 0;
                diagnostics.write().await.backfill_stalled = false;
                BACKFILL_BATCH_INTERVAL
            }
            // Nothing left to fill, ever: stop instead of waking every 10s to
            // re-check a frontier and floor that cannot change.
            Ok(BackfillProgress::Complete) => return,
            Ok(BackfillProgress::Waiting) => {
                idle_steps = idle_steps.saturating_add(1);
                if idle_steps == BACKFILL_STALL_STEPS {
                    // Report once on entering the stall rather than every tick.
                    warn!(
                        attempts = idle_steps,
                        "History backfill is not advancing: no peer is serving the range it needs. \
                         It keeps retrying; the frontier stays where it is."
                    );
                    diagnostics.write().await.backfill_stalled = true;
                }
                BACKFILL_IDLE_INTERVAL
            }
            Err(e) => {
                idle_steps = idle_steps.saturating_add(1);
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
    peers: &mut PeerHandler,
    store: &Store,
    blockchain: &Blockchain,
    config: &BackfillConfig,
    diagnostics: &Arc<tokio::sync::RwLock<SyncDiagnostics>>,
    reconciled: &mut bool,
    plan: &mut Option<BackfillPlan>,
) -> Result<BackfillProgress, SyncError> {
    // Only fill while the node is at the head. Gating on the snap flag alone was
    // not enough: it is clear on every restart of an already-synced node (the
    // auto-switch in `sync_manager`) and under `--syncmode full`, so backfill would
    // compete with a full-sync catch-up for the same peers, and would pin
    // `plan.head` to a stale head.
    if !blockchain.is_synced() {
        return Ok(BackfillProgress::Waiting);
    }

    // Without this, backfill would see `frontier == 0` on a node that snap-synced
    // before the frontier was tracked and conclude there is nothing to do.
    if !*reconciled {
        reconcile_history_bounds(store).await?;
        *reconciled = true;
    }

    // Resolve the floor and the tx-index reference head once per run.
    let plan = match plan {
        Some(plan) => plan,
        None => {
            let Some(floor) = resolve_floor(store, &config.mode).await? else {
                // `postmerge` on a chain that has not merged yet — keep idling,
                // since that can change.
                return Ok(BackfillProgress::Waiting);
            };
            plan.insert(BackfillPlan {
                floor,
                head: store.get_latest_block_number()?,
                mode_label: format!("{:?}", config.mode),
                first_batch: true,
            })
        }
    };
    let floor = plan.floor;

    let frontier = store.get_earliest_block_number().await?;
    {
        let mut diag = diagnostics.write().await;
        diag.backfill_mode = Some(plan.mode_label.clone());
        diag.backfill_floor = Some(floor);
        diag.backfill_frontier = Some(frontier);
        diag.backfill_complete = frontier <= floor;
    }
    // The first batch still has work to do at `frontier == floor`: that block is
    // the snap pivot, which has a body but no receipts (see below), so completing
    // here would leave the floor block permanently short of its receipts.
    if frontier <= floor && !plan.first_batch {
        info!(floor, "Historical chain backfill complete");
        return Ok(BackfillProgress::Complete);
    }
    if frontier < floor {
        // Already below the floor: nothing to repair even on the first batch.
        info!(floor, "Historical chain backfill complete");
        return Ok(BackfillProgress::Complete);
    }

    // Read headers top-down (highest first): the peer returns bodies/receipts in
    // request order, so a truncated response still yields a contiguous run adjacent
    // to the frontier, letting us lower the frontier without leaving a hole.
    //
    // The first batch includes the frontier block itself, every later batch stops
    // just below it. On a snap-synced node the frontier is the pivot, and snap only
    // stored the pivot's *body* (`add_block` writes no receipts and the block is
    // never executed), so its receipts would otherwise never be filled and
    // `eth_getBlockReceipts`/`eth_getTransactionReceipt` would stay wrong for that
    // one block while the frontier advertised it as complete. Re-fetching one body
    // is the cost of repairing it.
    let batch_hi = if plan.first_batch {
        frontier
    } else {
        frontier - 1
    };
    let batch_lo = batch_hi.saturating_sub(BACKFILL_BATCH_SIZE - 1).max(floor);
    debug_assert!(batch_lo <= batch_hi, "batch range must be non-empty");
    let mut headers: Vec<BlockHeader> = Vec::with_capacity((batch_hi - batch_lo + 1) as usize);
    for number in (batch_lo..=batch_hi).rev() {
        let header = store
            .get_block_header(number)?
            .ok_or(SyncError::CorruptDB)?;
        headers.push(header);
    }

    // Bodies and receipts are each validated against the headers inside the peer
    // request (block-body validation; receipts-root recomputed from logs, which
    // reconstructs the bloom omitted from eth/69 onward).
    //
    // Receipts are requested first because they are the likelier of the two to come
    // back short or not at all (they are far larger on the wire and fewer peers
    // retain them), and whatever the other request returned would be discarded and
    // re-fetched next round.
    let Some(receipts) = peers.request_receipts(&headers).await? else {
        return Ok(BackfillProgress::Waiting);
    };
    let Some(bodies) = peers.request_block_bodies(&headers).await? else {
        return Ok(BackfillProgress::Waiting);
    };

    // Only take blocks that have BOTH a body and receipts; because both responses
    // are prefixes of the top-down header list, their common prefix is contiguous
    // from the frontier downward.
    let filled = bodies.len().min(receipts.len());
    if filled == 0 {
        return Ok(BackfillProgress::Waiting);
    }

    let head = plan.head;
    let horizon = config.tx_index_horizon;
    let blocks: Vec<BackfilledBlock> = headers
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
        .collect();

    // `filled` blocks were stored counting down from `batch_hi`.
    let new_earliest = batch_hi + 1 - filled as u64;
    store.add_backfilled_blocks(blocks, new_earliest).await?;
    plan.first_batch = false;
    {
        let mut diag = diagnostics.write().await;
        diag.backfill_frontier = Some(new_earliest);
        diag.backfill_complete = new_earliest <= floor;
    }

    debug!(new_earliest, floor, "History backfill advanced");
    // The `frontier <= floor` check at the top of the next step logs completion and
    // stops the task; reporting `Advanced` here keeps that in one place.
    Ok(BackfillProgress::Advanced)
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

    use ethrex_common::types::{Block, BlockBody, LegacyTransaction, Transaction};
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

    /// A store still at genesis has nothing to reconcile against, and must not
    /// bisect an empty range.
    #[tokio::test]
    async fn reconcile_frontier_at_genesis_head_is_zero() {
        let store = Store::new("", EngineType::InMemory).expect("in-memory store");
        assert_eq!(reconcile_frontier(&store).await.unwrap(), 0);
    }

    /// If the head itself has no body the node isn't synced to the tip, so there is
    /// no contiguous run to measure; the frontier stays at the head rather than
    /// reporting a bogus lower value.
    #[tokio::test]
    async fn reconcile_frontier_without_a_body_at_head_returns_head() {
        // Headers 0..=100 canonical, but no bodies stored above genesis.
        let store = Store::new("", EngineType::InMemory).expect("in-memory store");
        let headers: Vec<BlockHeader> = (0..=100)
            .map(|number| BlockHeader {
                number,
                ..Default::default()
            })
            .collect();
        store.add_block_headers(headers.clone()).await.unwrap();
        let canonical: Vec<_> = headers.iter().map(|h| (h.number, h.hash())).collect();
        store
            .forkchoice_update(canonical, 100, headers[100].hash(), None, None)
            .await
            .unwrap();
        assert_eq!(reconcile_frontier(&store).await.unwrap(), 100);
    }

    /// A node snap-synced before the frontier was tracked has it at genesis.
    #[tokio::test]
    async fn reconcile_history_bounds_corrects_a_frontier_left_at_genesis() {
        let store = store_with_bodies_from(50, 100).await;
        store.update_earliest_block_number(0).await.unwrap();
        reconcile_history_bounds(&store).await.unwrap();
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 50);
        assert_eq!(store.get_tx_index_floor().await.unwrap(), None);
    }

    /// Bodies on `[earliest, head]`, indexed from `indexed_from`, with no floor
    /// recorded: a backfill that ran before the floor was tracked.
    async fn store_with_unindexed_backfill(
        earliest: u64,
        indexed_from: u64,
        head: u64,
        empty: &[u64],
    ) -> Store {
        store_with_tx_index(earliest, head, empty, |number| number >= indexed_from).await
    }

    /// Bodies on `[earliest, head]`, with the blocks `indexed` picks in the
    /// transaction index and no floor recorded.
    async fn store_with_tx_index(
        earliest: u64,
        head: u64,
        empty: &[u64],
        indexed: impl Fn(u64) -> bool,
    ) -> Store {
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
        for number in earliest..=head {
            let transactions = if empty.contains(&number) {
                vec![]
            } else {
                vec![Transaction::LegacyTransaction(LegacyTransaction {
                    nonce: number,
                    ..Default::default()
                })]
            };
            let body = BlockBody {
                transactions,
                ommers: vec![],
                withdrawals: Some(vec![]),
            };
            let header = headers[number as usize].clone();
            if indexed(number) {
                store.add_block(Block::new(header, body)).await.unwrap();
            } else {
                store.add_block_body(header.hash(), body).await.unwrap();
            }
        }
        store.update_earliest_block_number(earliest).await.unwrap();
        store
    }

    /// Independent of the current horizon: a node restarted without
    /// `--history.transactions` still has the unindexed blocks.
    #[tokio::test]
    async fn reconcile_history_bounds_records_a_missing_tx_index_floor() {
        // 44 and 45 are empty, so the highest unindexed block with transactions is 43.
        let store = store_with_unindexed_backfill(40, 45, 100, &[44, 45]).await;
        reconcile_history_bounds(&store).await.unwrap();
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 40);
        assert_eq!(store.get_tx_index_floor().await.unwrap(), Some(44));
    }

    #[tokio::test]
    async fn reconcile_history_bounds_leaves_a_fully_indexed_history_alone() {
        let store = store_with_unindexed_backfill(40, 40, 100, &[41]).await;
        reconcile_history_bounds(&store).await.unwrap();
        assert_eq!(store.get_tx_index_floor().await.unwrap(), None);
        assert!(store.history_bounds_reconciled().await.unwrap());
    }

    /// The search runs once per database: a later start trusts the recorded result.
    #[tokio::test]
    async fn reconcile_history_bounds_searches_the_tx_index_once() {
        let store = store_with_unindexed_backfill(40, 61, 100, &[41]).await;
        store.mark_history_bounds_reconciled(None).await.unwrap();
        reconcile_history_bounds(&store).await.unwrap();
        assert_eq!(store.get_tx_index_floor().await.unwrap(), None);
    }

    /// One run of unindexed blocks below the indexed ones, as a horizon that never
    /// grew leaves them: the floor is exact wherever the boundary falls in a window.
    #[tokio::test]
    async fn tx_index_floor_is_found_at_every_boundary() {
        for indexed_from in 40..=60 {
            let store = store_with_unindexed_backfill(40, indexed_from, 60, &[50, 51]).await;
            let expected = match indexed_from {
                51 | 52 => 50,
                other => other,
            };
            for window_size in [1, 3, 8, TX_INDEX_WINDOW] {
                let found = find_tx_index_floor(&store, window_size).await.unwrap();
                assert_eq!(
                    found, expected,
                    "indexed from {indexed_from}, window size {window_size}"
                );
            }
        }
    }

    /// A horizon raised between two runs: the later run indexed blocks below the
    /// ones the earlier run left out. Wherever those lie, a stretch as long as a
    /// window is found, and the floor is right above its highest block with
    /// transactions. The empty blocks cover a whole window and reach into the
    /// windows next to it, so the stretch is found from either side of them.
    #[tokio::test]
    async fn tx_index_floor_is_found_above_blocks_indexed_by_a_later_run() {
        let (earliest, head, window_size) = (10, 60, 8);
        let empty_runs: [&[u64]; 2] = [&[], &[27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38]];
        for empty in empty_runs {
            for unindexed_from in earliest..=head + 1 - window_size {
                let unindexed = unindexed_from..unindexed_from + window_size;
                let store = store_with_tx_index(earliest, head, empty, |number| {
                    !unindexed.contains(&number)
                })
                .await;
                let expected = unindexed
                    .clone()
                    .rev()
                    .find(|number| !empty.contains(number))
                    .map_or(earliest, |number| number + 1);
                let found = find_tx_index_floor(&store, window_size).await.unwrap();
                assert_eq!(found, expected, "unindexed {unindexed:?}, empty {empty:?}");
            }
        }
    }

    /// The floor is above every unindexed stretch, not only the lowest.
    #[tokio::test]
    async fn tx_index_floor_is_above_the_highest_unindexed_stretch() {
        let (earliest, head, window_size) = (10, 60, 8);
        let store = store_with_tx_index(earliest, head, &[], |number| {
            !(12..20).contains(&number) && !(40..48).contains(&number)
        })
        .await;
        assert_eq!(find_tx_index_floor(&store, window_size).await.unwrap(), 48);
    }

    /// What the windows cost: a stretch shorter than a window is found only when
    /// it holds the lowest or the highest block with transactions of a window.
    /// With the head at 60 and windows of 8, blocks 45 to 52 are one window, and
    /// every block here has transactions.
    #[tokio::test]
    async fn tx_index_floor_misses_a_short_stretch_inside_a_window() {
        let (earliest, head, window_size) = (10, 60, 8);
        let floor_with_unindexed = async |unindexed: std::ops::Range<u64>| {
            let store =
                store_with_tx_index(earliest, head, &[], |number| !unindexed.contains(&number))
                    .await;
            find_tx_index_floor(&store, window_size).await.unwrap()
        };
        assert_eq!(floor_with_unindexed(50..53).await, 53);
        assert_eq!(floor_with_unindexed(45..48).await, 48);
        assert_eq!(floor_with_unindexed(47..50).await, earliest);
    }

    /// A height without a canonical block is one a forkchoice update is rewriting.
    /// It is not read as unindexed: the search fails and the database stays
    /// unmarked, so the reconciliation runs again.
    #[tokio::test]
    async fn tx_index_floor_is_not_taken_from_a_canonical_chain_being_rewritten() {
        // The blocks above 55 are empty, so the search reads down to it.
        let store = store_with_tx_index(10, 60, &[56, 57, 58, 59, 60], |_| true).await;
        store.remove_block(55).await.unwrap();
        let found = find_tx_index_floor(&store, 8).await;
        assert!(
            matches!(found, Err(SyncError::CanonicalChainMoved(55))),
            "{found:?}"
        );

        assert!(reconcile_history_bounds(&store).await.is_err());
        assert_eq!(store.get_tx_index_floor().await.unwrap(), None);
        assert!(!store.history_bounds_reconciled().await.unwrap());

        // A head lowered below `earliest` between the two reads.
        let store = store_with_tx_index(10, 60, &[], |_| true).await;
        store.update_earliest_block_number(61).await.unwrap();
        let found = find_tx_index_floor(&store, 8).await;
        assert!(
            matches!(found, Err(SyncError::CanonicalChainMoved(60))),
            "{found:?}"
        );
    }

    /// A node without a sync manager reconciles from startup, and a cancelled
    /// token stops it.
    #[tokio::test]
    async fn reconcile_history_bounds_at_startup_marks_the_database() {
        let store = store_with_unindexed_backfill(40, 45, 100, &[]).await;
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        reconcile_history_bounds_at_startup(store.clone(), cancelled).await;
        assert!(!store.history_bounds_reconciled().await.unwrap());

        reconcile_history_bounds_at_startup(store.clone(), CancellationToken::new()).await;
        assert!(store.history_bounds_reconciled().await.unwrap());
        assert_eq!(store.get_tx_index_floor().await.unwrap(), Some(45));
    }

    // --- resolve_floor: how each mode picks the block to stop at ---

    /// An explicit block number is used verbatim, so an operator can keep just a
    /// recent slice of history instead of the whole post-merge range.
    #[tokio::test]
    async fn resolve_floor_honours_an_explicit_block() {
        let store = store_with_bodies_from(90, 100).await;
        let floor = resolve_floor(&store, &HistoryChain::Block(22_000_000))
            .await
            .unwrap();
        assert_eq!(floor, Some(22_000_000));
    }

    /// An explicit floor below the merge block is still honoured (best-effort),
    /// not silently clamped up to the merge block.
    #[tokio::test]
    async fn resolve_floor_honours_a_pre_merge_block() {
        let store = store_with_bodies_from(90, 100).await;
        let floor = resolve_floor(&store, &HistoryChain::Block(5))
            .await
            .unwrap();
        assert_eq!(floor, Some(5));
    }

    #[tokio::test]
    async fn resolve_floor_is_genesis_for_all_and_none_for_off() {
        let store = store_with_bodies_from(90, 100).await;
        assert_eq!(
            resolve_floor(&store, &HistoryChain::All).await.unwrap(),
            Some(0)
        );
        assert_eq!(
            resolve_floor(&store, &HistoryChain::Off).await.unwrap(),
            None
        );
    }

    /// Builds an in-memory store whose chain config comes from `genesis`, so
    /// config-driven paths (`merge_netsplit_block`, `byzantium_block`) can be
    /// exercised.
    async fn store_with_config(
        configure: impl FnOnce(&mut ethrex_common::types::Genesis),
    ) -> Store {
        let mut store = Store::new("", EngineType::InMemory).expect("in-memory store");
        let mut genesis = ethrex_common::types::Genesis::default();
        configure(&mut genesis);
        store
            .add_initial_state(genesis)
            .await
            .expect("load genesis");
        store
    }

    /// The `merge_netsplit_block` short-circuit, not the bisect: this is the branch
    /// networks that configure a merge block (e.g. Sepolia, Hoodi) actually take,
    /// and it must be used verbatim without touching headers.
    #[tokio::test]
    async fn resolve_postmerge_floor_uses_configured_merge_block() {
        let store = store_with_config(|g| {
            g.config.merge_netsplit_block = Some(1_735_371);
            g.config.byzantium_block = Some(0);
        })
        .await;
        assert_eq!(
            resolve_postmerge_floor(&store).await.unwrap(),
            Some(1_735_371),
            "a configured merge block must short-circuit the bisect"
        );
    }

    /// Pre-Byzantium receipts carry a post-state root instead of a status flag and
    /// cannot be decoded, so a floor below Byzantium is clamped up to it rather
    /// than driving requests that can only fail.
    #[tokio::test]
    async fn resolve_floor_clamps_below_byzantium() {
        let store = store_with_config(|g| g.config.byzantium_block = Some(4_370_000)).await;
        assert_eq!(
            resolve_floor(&store, &HistoryChain::Block(1_000_000))
                .await
                .unwrap(),
            Some(4_370_000),
            "an explicit pre-Byzantium floor must be clamped"
        );
        assert_eq!(
            resolve_floor(&store, &HistoryChain::All).await.unwrap(),
            Some(4_370_000),
            "`all` must stop at Byzantium, not genesis"
        );
        // A floor above Byzantium is untouched.
        assert_eq!(
            resolve_floor(&store, &HistoryChain::Block(22_000_000))
                .await
                .unwrap(),
            Some(22_000_000)
        );
    }

    /// A chain with no Byzantium block configured is post-Byzantium from genesis,
    /// so `all` may legitimately reach block 0.
    #[tokio::test]
    async fn resolve_floor_allows_genesis_without_byzantium_config() {
        let store = store_with_config(|_| {}).await;
        assert_eq!(
            resolve_floor(&store, &HistoryChain::All).await.unwrap(),
            Some(0)
        );
    }

    /// `postmerge` on a chain that never merged (all headers carry PoW
    /// difficulty) has no post-merge segment, so there is nothing to fill.
    #[tokio::test]
    async fn resolve_floor_is_none_for_postmerge_on_an_unmerged_chain() {
        let store = Store::new("", EngineType::InMemory).expect("in-memory store");
        let headers: Vec<BlockHeader> = (0..=10)
            .map(|number| BlockHeader {
                number,
                difficulty: ethrex_common::U256::from(1_000_000u64),
                ..Default::default()
            })
            .collect();
        store.add_block_headers(headers.clone()).await.unwrap();
        let canonical: Vec<_> = headers.iter().map(|h| (h.number, h.hash())).collect();
        store
            .forkchoice_update(canonical, 10, headers[10].hash(), None, None)
            .await
            .unwrap();
        assert_eq!(
            resolve_floor(&store, &HistoryChain::PostMerge)
                .await
                .unwrap(),
            None
        );
    }
}
