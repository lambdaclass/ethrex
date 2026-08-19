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
    if frontier <= floor {
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
