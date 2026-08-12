//! History pruner task: deletes block bodies, receipts, transaction
//! locations, and non-canonical block data for heights older than the
//! configured retention window.

use crate::error::StoreError;
use crate::store::Store;
use ethrex_common::types::BlockNumber;
use std::future::Future;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;

#[cfg(feature = "metrics")]
use ethrex_metrics::pruning::METRICS_PRUNING;

/// How often a pruner loop re-checks the retention window. Shared by the L1 and
/// L2 entry points ([`HistoryPruner::run`] and
/// [`HistoryPruner::run_with_floor`]) so the two cadences cannot drift.
pub const PRUNE_INTERVAL_SECS: u64 = 12;
const PRUNE_PASS_TIMEOUT_MS: u64 = 2_000;
// Heights deleted per pass. The key count scales with tx density: at ~70
// tx/block (Ethereum mainnet) a full pass deletes ~600K keys (4096 × (1 body +
// receipts + tx_locations)); on denser chains like BSC (~200 tx/block) it's
// ~1.6M.
//
// The trade-off in both directions is real: larger batches outrun RocksDB
// compaction and trigger write stalls, smaller ones can fall behind block sync.
// 4096 is a starting point chosen from that reasoning, not a measured ceiling —
// treat it as tunable and re-measure before assuming it is safe on a chain
// denser than BSC.
const PRUNE_BATCH_SIZE: usize = 4_096;
// Heights per `prune_block_heights` call within a pass. This bounds peak heap and
// gives the pass deadline a chunk boundary to fire at.
//
// Decoded bodies are *not* held across the call: the gather phase derives each
// body's receipt keys and transaction hashes and drops the body immediately (see
// `BodyDeletions`). What a chunk retains until commit is those derived keys plus the
// transaction-location edits, on the order of a hundred bytes per transaction — a few
// tens of MB at 256 heights even on a dense chain. Raising this raises that
// proportionally, and also widens the `TRANSACTION_LOCATIONS` batch read the write
// phase issues.
const PRUNE_CHUNK_SIZE: usize = 256;
// Used as the prune floor when no `FinalizedBlockNumber` is set (chains
// without engine-API finality, or a node before its first FCU). Covers
// reorg depths far beyond mainnet norms while letting the pruner do
// useful work.
const SAFETY_DISTANCE: u64 = 256;
// A near-head margin the pruner always keeps: enough to avoid pruning the
// head's own body (which would stall block production) and to cover reorg
// depth when finality is unavailable (matches `SAFETY_DISTANCE`).
//
// This is NOT what protects the state-regeneration window. On restart the node
// re-executes every block from the last *persisted* state root up to the head
// (`regenerate_head_state`). Today both commit gates bound that window to
// ~`DB_COMMIT_THRESHOLD` (128) — the depth gate is passed `DB_COMMIT_THRESHOLD`
// by every `add_block_pipeline_bounded` caller including the bulk-sync path
// (`Blockchain::add_blocks_in_batch`), and the canonical safe-commit gate tracks
// `head - DB_COMMIT_THRESHOLD` — so `KEEP_RECENT` happens to cover it with a 2x
// margin. That is a coincidence of the current constants, not a guarantee:
// `tick` caps the prune target at the actual persisted height
// (`Store::get_latest_persisted_state_block`) so the invariant holds even if the
// window widens.
const KEEP_RECENT: u64 = 256;

// The pruner never prunes below this height. Block 0's body is cheap to keep and
// `eth_getBlockByNumber(0)` is a common way to identify a chain; geth and reth
// both retain it. Without this floor the first pass deletes it and genesis
// lookups start returning null.
const LOWEST_PRUNABLE_BLOCK: u64 = 1;

pub struct HistoryPruner {
    store: Store,
    retention: Duration,
    /// Heights within this distance of the head are never pruned. Defaults to
    /// [`KEEP_RECENT`]; only tests override it (to exercise the floor without
    /// building 256-block chains).
    keep_recent: u64,
    /// Test-only override for the persisted-state floor (see [`Self::tick`]).
    /// `None` in production, where the floor is read from the store. Synthetic
    /// test chains have no persisted trie state, so the production query would
    /// return 0 and block all pruning; tests set this to isolate the
    /// retention/finality logic (or to a specific value to exercise the floor).
    persisted_floor_override: Option<u64>,
}

impl HistoryPruner {
    pub fn new(store: Store, retention: Duration) -> Self {
        Self {
            store,
            retention,
            keep_recent: KEEP_RECENT,
            persisted_floor_override: None,
        }
    }

    /// Run until `cancel` fires. Every [`PRUNE_INTERVAL_SECS`], run one pass.
    /// Errors are logged at ERROR level and don't stop the loop.
    ///
    /// Cancellation stops further passes from *starting*. It does not abort a pass
    /// already in flight: the `tick` below is awaited inside this branch, so once a
    /// pass begins it runs to completion (bounded by [`PRUNE_PASS_TIMEOUT_MS`] plus
    /// one chunk). Since the pruner commits through its own write transactions
    /// rather than the persist worker, a pass overlapping `Store::shutdown` can
    /// still land its batch after the final fsync; that batch is atomic and is
    /// replayed from the WAL on the next start, so the cost is recovery time, not
    /// consistency. Callers that want the stronger guarantee must also await the
    /// task (e.g. `TaskTracker::wait`) before calling `Store::shutdown`.
    pub async fn run(self, cancel: CancellationToken) {
        self.run_inner(cancel, || async { Ok(None) }, false).await
    }

    /// Like [`Self::run`], but consults `max_prunable` before every pass to obtain
    /// an additional upper bound on what may be deleted.
    ///
    /// This is the L2 entry point: there, pruning must additionally stop at the last
    /// block committed to L1, because the committer and prover still read the bodies
    /// of uncommitted blocks. Sharing this loop with [`Self::run`] keeps the
    /// interval and the cancellation semantics identical for both node types.
    ///
    /// A provider returning `Ok(None)` means "no cap is currently known", which is
    /// treated as *withhold everything*: the pass is skipped rather than run
    /// uncapped. An `Err` is logged and likewise skips the pass.
    pub async fn run_with_floor<F, Fut>(self, cancel: CancellationToken, max_prunable: F)
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<Option<BlockNumber>, StoreError>>,
    {
        self.run_inner(cancel, max_prunable, true).await
    }

    /// Shared loop behind [`Self::run`] and [`Self::run_with_floor`].
    ///
    /// `floor_required` distinguishes "this node has no additional cap" (L1, where
    /// `None` legitimately means unconstrained) from "this node's cap could not be
    /// determined" (L2, where `None` must withhold everything).
    async fn run_inner<F, Fut>(
        self,
        cancel: CancellationToken,
        max_prunable: F,
        floor_required: bool,
    ) where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<Option<BlockNumber>, StoreError>>,
    {
        // Force metric registration up front so the pruning series exist from
        // startup. They are otherwise first touched inside a productive pass, which
        // leaves an operator unable to tell "pruner idle" from "pruner not running".
        #[cfg(feature = "metrics")]
        std::sync::LazyLock::force(&METRICS_PRUNING);

        let mut interval = tokio::time::interval(Duration::from_secs(PRUNE_INTERVAL_SECS));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::debug!("history pruner shutting down");
                    return;
                }
                _ = interval.tick() => {
                    let floor = match max_prunable().await {
                        Ok(floor) => floor,
                        Err(e) => {
                            tracing::error!(error = ?e, "history pruner: could not determine the prune cap; skipping pass");
                            continue;
                        }
                    };
                    // `tick_with_floor` treats `None` as "no cap", so a provider that
                    // cannot answer must not reach it: skip instead of pruning uncapped.
                    if floor_required && floor.is_none() {
                        continue;
                    }
                    // Errors are already logged with their range by `tick_with_floor`.
                    let _ = self.tick_with_floor(now_seconds(), floor).await;
                }
            }
        }
    }

    /// Run one pass. Returns the number of heights processed.
    /// Public for testability (lets tests inject `now`).
    pub async fn tick(&self, now_secs: u64) -> Result<usize, StoreError> {
        self.tick_with_floor(now_secs, None).await
    }

    /// Run one pass, with an optional caller-supplied cap: `max_prunable` is the
    /// highest height that may be deleted. Heights above it are left alone.
    ///
    /// Used by the L2 node to withhold blocks not yet committed to L1 — their
    /// bodies are still needed to build and prove the batch. There, `max_prunable`
    /// is the last block of the newest batch that has a commit transaction.
    pub async fn tick_with_floor(
        &self,
        now_secs: u64,
        max_prunable: Option<BlockNumber>,
    ) -> Result<usize, StoreError> {
        // Empty / pre-init store: nothing to prune. Bail before touching any
        // downstream reads so we don't surface MissingEarliestBlockNumber from
        // `find_canonical_block_by_timestamp`.
        let stored_earliest = match self.store.get_earliest_block_number().await {
            Ok(n) => n,
            Err(StoreError::MissingEarliestBlockNumber) => return Ok(0),
            Err(e) => return Err(e),
        };
        // Keep the genesis body. `EarliestBlockNumber` still advances past it, so
        // block 0 becomes a hole below the reported range — harmless, because the
        // block-by-number RPC path reads header+body directly rather than gating
        // on the pointer, and genesis carries no logs or transactions to find.
        let mut earliest = stored_earliest.max(LOWEST_PRUNABLE_BLOCK);

        // Never prune within `keep_recent` of the head: the state DB persists
        // ~128 blocks behind the head and re-executes the rest from their
        // bodies on restart, so pruning inside that window bricks the node and
        // pruning the head's body stalls block production. If the chain is
        // shorter than `keep_recent`, there is nothing safe to prune yet.
        let head = match self.store.get_latest_block_number().await {
            Ok(n) => n,
            Err(StoreError::MissingLatestBlockNumber) => return Ok(0),
            Err(e) => return Err(e),
        };
        let Some(prune_ceiling) = head.checked_sub(self.keep_recent) else {
            return Ok(0);
        };

        // Prefer FinalizedBlockNumber as the prune floor. Chains without
        // engine-API finality (e.g. BSC PoSA) never write it; fall back to
        // `head - SAFETY_DISTANCE` so the pruner can still do useful work.
        let finalized = match self.store.get_finalized_block_number().await? {
            Some(n) => n,
            None => head.saturating_sub(SAFETY_DISTANCE),
        };

        let target_ts = now_secs.saturating_sub(self.retention.as_secs());
        let retention_block = match self
            .store
            .find_canonical_block_by_timestamp(target_ts, finalized)
            .await?
        {
            Some(n) => n,
            None => return Ok(0),
        };

        // Never prune at or above the last block whose state trie is persisted
        // to disk: on restart `regenerate_head_state` re-executes every block
        // above that point from its body. `KEEP_RECENT` covers the current
        // ~DB_COMMIT_THRESHOLD window, but we cap at the measured persisted
        // height so the invariant survives any future widening of it.
        let persisted = match self.persisted_floor_override {
            Some(p) => p,
            None => self.store.get_latest_persisted_state_block().await?,
        };

        // Cap by every floor: finality, the retention window, the near-head
        // margin, the persisted-state boundary, and any caller-supplied cap
        // (L2: last block committed to L1).
        let mut target = finalized
            .min(retention_block)
            .min(prune_ceiling)
            .min(persisted);
        if let Some(max_prunable) = max_prunable {
            target = target.min(max_prunable);
        }

        // Publish the position gauges before the caught-up check, not after. These
        // are the two series an operator watches to answer "is pruning keeping up",
        // and the common case is a pass that finds nothing to do — reporting only on
        // productive passes leaves them frozen at the last productive value and makes
        // an idle pruner look identical to a wedged one.
        #[cfg(feature = "metrics")]
        {
            METRICS_PRUNING.prune_target_block.set(target as i64);
            METRICS_PRUNING
                .prune_lag_blocks
                .set(target.saturating_sub(earliest) as i64);
            METRICS_PRUNING.earliest_block_number.set(earliest as i64);
        }

        if earliest > target {
            return Ok(0);
        }

        let start = Instant::now();

        let deadline = start + Duration::from_millis(PRUNE_PASS_TIMEOUT_MS);
        let mut processed: usize = 0;

        // One chunk per loop iteration: gather fans out across rayon threads, then a
        // single write txn commits that chunk's deletes (including its
        // EarliestBlockNumber advance, so a pass cut short by the deadline resumes
        // where it stopped). The pass ends when the target is reached,
        // PRUNE_BATCH_SIZE heights have been processed, or the deadline elapses
        // between chunks.
        //
        // `isolating` handles a poisoned height. `prune_block_heights` is
        // all-or-nothing and only advances the stored pointer on success, so without
        // this a single undecodable body would abort every pass at the same offset
        // forever, silently halting all pruning. On failure we retry the range one
        // height at a time; a height that fails alone is logged and stepped over so
        // the rest of the backlog still drains.
        let mut isolating = false;
        while earliest <= target && processed < PRUNE_BATCH_SIZE && Instant::now() < deadline {
            let remaining_budget = PRUNE_BATCH_SIZE - processed;
            let remaining_target = (target + 1 - earliest) as usize;
            let mut chunk = PRUNE_CHUNK_SIZE.min(remaining_budget).min(remaining_target);
            if isolating {
                chunk = 1;
            }
            if chunk == 0 {
                break;
            }
            let _counts = match self.store.prune_block_heights(earliest, chunk).await {
                Ok(counts) => {
                    isolating = false;
                    counts
                }
                Err(e) if chunk > 1 => {
                    tracing::warn!(
                        error = ?e, start = earliest, chunk,
                        "history pruner: chunk failed; retrying one height at a time to isolate it"
                    );
                    isolating = true;
                    continue;
                }
                Err(e) => {
                    // A single height cannot be pruned. Step over it rather than
                    // retrying it forever; its data stays on disk (a bounded leak we
                    // count) while the pointer advances so later heights are freed.
                    tracing::error!(
                        error = ?e, height = earliest,
                        "history pruner: skipping height that cannot be pruned; its bodies/receipts will not be reclaimed"
                    );
                    #[cfg(feature = "metrics")]
                    METRICS_PRUNING.heights_skipped.inc();
                    self.store
                        .advance_earliest_block_number(earliest + 1)
                        .await?;
                    earliest += 1;
                    processed += 1;
                    isolating = false;
                    continue;
                }
            };
            #[cfg(feature = "metrics")]
            {
                METRICS_PRUNING.bodies_deleted.inc_by(_counts.bodies);
                METRICS_PRUNING.receipts_deleted.inc_by(_counts.receipts);
                METRICS_PRUNING
                    .tx_locations_deleted
                    .inc_by(_counts.tx_locations);
                METRICS_PRUNING
                    .orphan_headers_deleted
                    .inc_by(_counts.orphan_headers);
                METRICS_PRUNING
                    .index_entries_deleted
                    .inc_by(_counts.index_entries);
            }
            earliest += chunk as u64;
            processed += chunk;
        }

        #[cfg(feature = "metrics")]
        {
            let duration_ms = start.elapsed().as_millis() as f64;
            METRICS_PRUNING.pass_duration_ms.observe(duration_ms);
            METRICS_PRUNING.pass_blocks.observe(processed as f64);
            METRICS_PRUNING.earliest_block_number.set(earliest as i64);
            METRICS_PRUNING
                .prune_lag_blocks
                .set(target.saturating_sub(earliest) as i64);
        }

        Ok(processed)
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EngineType;
    use ethrex_common::types::{Block, BlockBody, BlockHeader};

    fn header_with_ts(n: u64, ts: u64, parent: ethrex_common::H256) -> BlockHeader {
        BlockHeader {
            number: n,
            timestamp: ts,
            parent_hash: parent,
            ..Default::default()
        }
    }

    /// Build a pruner with an explicit `keep_recent`, so tests can exercise the
    /// head floor without constructing 256-block chains.
    fn pruner_with_keep_recent(store: Store, secs: u64, keep_recent: u64) -> HistoryPruner {
        HistoryPruner {
            store,
            retention: Duration::from_secs(secs),
            keep_recent,
            // Synthetic test chains have no persisted trie state, so the
            // production persisted-floor query would return 0 and block all
            // pruning. Disable it here; the floor has its own test below
            // (`tick_keeps_unpersisted_state_window`).
            persisted_floor_override: Some(u64::MAX),
        }
    }

    /// Regression for the floor being silently inert: the *production* query must
    /// report the on-disk boundary, not the head.
    ///
    /// `Store::has_state_root` reads through the in-memory `TrieLayerCache` before
    /// disk, and every block rewrites the trie root node, so asking it about the
    /// head's state root answers "yes" from RAM on any live node. A floor built on
    /// it collapses to `head` and constrains nothing. This exercises
    /// `get_latest_persisted_state_block` directly, with no override, on a chain
    /// whose state was never committed: the honest answer is 0.
    #[tokio::test]
    async fn persisted_state_block_reports_disk_not_layer_cache() {
        let store = Store::new("", EngineType::InMemory).unwrap();

        let mut parent = ethrex_common::H256::zero();
        for n in 0..=5u64 {
            let h = header_with_ts(n, n * 100, parent);
            let hash = h.hash();
            store
                .add_block(Block {
                    header: h,
                    body: BlockBody::default(),
                })
                .await
                .unwrap();
            store.set_canonical_block_for_test(n, hash).await.unwrap();
            parent = hash;
        }
        store.set_latest_block_number_for_test(5).await.unwrap();

        // These synthetic headers carry the default (empty) state root and no trie
        // was ever written, so nothing above genesis is persisted. Returning 5 here
        // would mean the query is answering from the layer cache / treating the head
        // as durable.
        let persisted = store.get_latest_persisted_state_block().await.unwrap();
        assert_eq!(
            persisted, 0,
            "persisted floor must reflect on-disk state, not the head"
        );
    }

    /// With no override, a chain that has never committed state must not be pruned
    /// at all — the persisted floor pins the target at 0.
    #[tokio::test]
    async fn tick_without_override_is_blocked_by_persisted_floor() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        store.advance_earliest_block_number(0).await.unwrap();

        let mut parent = ethrex_common::H256::zero();
        for n in 0..=20u64 {
            let h = header_with_ts(n, n * 100, parent);
            let hash = h.hash();
            store
                .add_block(Block {
                    header: h,
                    body: BlockBody::default(),
                })
                .await
                .unwrap();
            store.set_canonical_block_for_test(n, hash).await.unwrap();
            parent = hash;
        }
        store.set_finalized_block_number_for_test(20).await.unwrap();
        store.set_latest_block_number_for_test(20).await.unwrap();

        // Retention and finality both reach the head; only the persisted floor
        // stands in the way. `HistoryPruner::new` leaves it enabled.
        let pruner = HistoryPruner {
            store: store.clone(),
            retention: Duration::from_secs(1),
            keep_recent: 0,
            persisted_floor_override: None,
        };
        assert_eq!(pruner.tick(10_000).await.unwrap(), 0);
        for n in 0..=20 {
            assert!(
                store.get_block_body(n).await.unwrap().is_some(),
                "body {n} must survive: no state is persisted"
            );
        }
    }

    /// `max_prunable` (used by the L2 node to withhold uncommitted blocks) caps the
    /// target independently of the other floors.
    #[tokio::test]
    async fn tick_respects_max_prunable_cap() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        store.advance_earliest_block_number(0).await.unwrap();

        let mut parent = ethrex_common::H256::zero();
        for n in 0..=20u64 {
            let h = header_with_ts(n, n * 100, parent);
            let hash = h.hash();
            store
                .add_block(Block {
                    header: h,
                    body: BlockBody::default(),
                })
                .await
                .unwrap();
            store.set_canonical_block_for_test(n, hash).await.unwrap();
            parent = hash;
        }
        store.set_finalized_block_number_for_test(20).await.unwrap();
        store.set_latest_block_number_for_test(20).await.unwrap();

        // Everything is old enough and the other floors are disabled, but only
        // blocks up to 6 are "committed".
        let pruner = pruner_with_keep_recent(store.clone(), 1, 0);
        let pruned = pruner.tick_with_floor(10_000, Some(6)).await.unwrap();

        assert_eq!(pruned, 6, "heights 1..=6 (genesis is never pruned)");
        for n in 1..=6 {
            assert!(store.get_block_body(n).await.unwrap().is_none(), "body {n}");
        }
        for n in 7..=20 {
            assert!(
                store.get_block_body(n).await.unwrap().is_some(),
                "body {n} is not committed yet"
            );
        }
    }

    /// The genesis body is never pruned: `eth_getBlockByNumber(0)` is a common
    /// chain-identity probe and geth/reth both keep it.
    #[tokio::test]
    async fn tick_never_prunes_genesis_body() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        store.advance_earliest_block_number(0).await.unwrap();

        let mut parent = ethrex_common::H256::zero();
        for n in 0..=10u64 {
            let h = header_with_ts(n, n * 100, parent);
            let hash = h.hash();
            store
                .add_block(Block {
                    header: h,
                    body: BlockBody::default(),
                })
                .await
                .unwrap();
            store.set_canonical_block_for_test(n, hash).await.unwrap();
            parent = hash;
        }
        store.set_finalized_block_number_for_test(10).await.unwrap();
        store.set_latest_block_number_for_test(10).await.unwrap();

        let pruner = pruner_with_keep_recent(store.clone(), 1, 0);
        pruner.tick(10_000).await.unwrap();

        assert!(
            store.get_block_body(0).await.unwrap().is_some(),
            "genesis body must survive pruning"
        );
        assert!(
            store.get_block_body(1).await.unwrap().is_none(),
            "block 1 should still be pruned"
        );
    }

    /// A store with no `EarliestBlockNumber` at all (never initialised via
    /// `add_initial_state`) must no-op rather than surfacing
    /// `MissingEarliestBlockNumber`.
    ///
    /// Note this exits at the very first guard, so it deliberately does *not* cover
    /// the no-finality fallback — `tick_falls_back_to_safety_distance_without_finality`
    /// below does that.
    #[tokio::test]
    async fn tick_uninitialised_store_no_work() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        let pruner = HistoryPruner::new(store, Duration::from_secs(3600));
        let done = pruner.tick(10_000).await.unwrap();
        assert_eq!(done, 0);
    }

    /// Chains without engine-API finality (BSC PoSA, and any node before its first
    /// FCU) never write `FinalizedBlockNumber`. The pruner must then fall back to
    /// `head - SAFETY_DISTANCE` as the floor, and that fallback must actually bound
    /// the target — otherwise pruning either never progresses on those chains or
    /// reaches inside the reorg window.
    ///
    /// This is the only test that reaches the `None` branch of the finality read: it
    /// is the one path where `set_finalized_block_number_for_test` must NOT be called.
    #[tokio::test]
    async fn tick_falls_back_to_safety_distance_without_finality() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        store.advance_earliest_block_number(0).await.unwrap();

        // A chain long enough that `head - SAFETY_DISTANCE` is a meaningful floor
        // strictly inside it.
        let head = SAFETY_DISTANCE + 50;
        let mut parent = ethrex_common::H256::zero();
        for n in 0..=head {
            let h = header_with_ts(n, n * 100, parent);
            let hash = h.hash();
            store
                .add_block(Block {
                    header: h,
                    body: BlockBody::default(),
                })
                .await
                .unwrap();
            store.set_canonical_block_for_test(n, hash).await.unwrap();
            parent = hash;
        }
        store.set_latest_block_number_for_test(head).await.unwrap();
        // Deliberately no `set_finalized_block_number_for_test`.
        assert!(
            store.get_finalized_block_number().await.unwrap().is_none(),
            "this test is only meaningful with finality unset"
        );

        // keep_recent = 0 and a 1s retention isolate the fallback: every other floor
        // reaches the head, so `target` can only be `head - SAFETY_DISTANCE` = 50.
        let pruner = pruner_with_keep_recent(store.clone(), 1, 0);
        let pruned = pruner.tick(10_000_000).await.unwrap();

        assert_eq!(
            pruned, 50,
            "should prune heights 1..=50 (head - SAFETY_DISTANCE), genesis retained"
        );
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 51);
        for n in 1..=50 {
            assert!(store.get_block_body(n).await.unwrap().is_none(), "body {n}");
        }
        // The SAFETY_DISTANCE window above the floor is untouched.
        for n in 51..=head {
            assert!(
                store.get_block_body(n).await.unwrap().is_some(),
                "body {n} is inside the reorg-safety window"
            );
        }
    }

    /// The production configuration, end to end: `HistoryPruner::new` with no
    /// overrides, so `KEEP_RECENT = 256` and the real
    /// `get_latest_persisted_state_block` are both live and both must be respected
    /// simultaneously.
    ///
    /// Every other pruning test neutralises at least one floor to isolate another, so
    /// without this one nothing exercises the composed `min(...)` that actually runs
    /// on a node, and nothing catches the persisted-floor query being wired in wrong.
    #[tokio::test]
    async fn tick_with_production_floors_prunes_up_to_persisted_state() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        store.advance_earliest_block_number(0).await.unwrap();

        // Long enough that KEEP_RECENT is not the binding floor.
        let head = KEEP_RECENT + 400;
        let mut parent = ethrex_common::H256::zero();
        for n in 0..=head {
            let h = header_with_ts(n, n * 100, parent);
            let hash = h.hash();
            store
                .add_block(Block {
                    header: h,
                    body: BlockBody::default(),
                })
                .await
                .unwrap();
            store.set_canonical_block_for_test(n, hash).await.unwrap();
            parent = hash;
        }
        store
            .set_finalized_block_number_for_test(head)
            .await
            .unwrap();
        store.set_latest_block_number_for_test(head).await.unwrap();

        // No overrides: this is exactly what `init_l1` constructs.
        let pruner = HistoryPruner::new(store.clone(), Duration::from_secs(1));

        // These synthetic headers carry the default (empty) state root and no trie was
        // ever committed, so the honest persisted height is 0 and it is the binding
        // floor. Pruning must therefore do nothing at all, even though finality, the
        // retention window and the near-head margin would all permit it.
        assert_eq!(store.get_latest_persisted_state_block().await.unwrap(), 0);
        assert_eq!(
            pruner.tick(10_000_000).await.unwrap(),
            0,
            "the persisted-state floor must veto pruning even when every other floor allows it"
        );
        for n in 0..=head {
            assert!(
                store.get_block_body(n).await.unwrap().is_some(),
                "body {n} must survive: no state is persisted"
            );
        }
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 0);
    }

    /// The near-head margin and the persisted-state floor binding at the same time:
    /// whichever is lower must win. Guards against a future edit reordering or
    /// dropping a term from the `min(...)` chain, which no single-floor test would
    /// notice.
    #[tokio::test]
    async fn tick_takes_the_lowest_of_two_live_floors() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        store.advance_earliest_block_number(0).await.unwrap();

        let mut parent = ethrex_common::H256::zero();
        for n in 0..=40u64 {
            let h = header_with_ts(n, n * 100, parent);
            let hash = h.hash();
            store
                .add_block(Block {
                    header: h,
                    body: BlockBody::default(),
                })
                .await
                .unwrap();
            store.set_canonical_block_for_test(n, hash).await.unwrap();
            parent = hash;
        }
        store.set_finalized_block_number_for_test(40).await.unwrap();
        store.set_latest_block_number_for_test(40).await.unwrap();

        // keep_recent = 10 → prune_ceiling = 30; persisted = 25. The lower one wins.
        let pruner = HistoryPruner {
            store: store.clone(),
            retention: Duration::from_secs(1),
            keep_recent: 10,
            persisted_floor_override: Some(25),
        };
        assert_eq!(
            pruner.tick(10_000).await.unwrap(),
            25,
            "persisted floor wins"
        );
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 26);

        // And with the two swapped, the other one wins.
        let store2 = Store::new("", EngineType::InMemory).unwrap();
        store2.advance_earliest_block_number(0).await.unwrap();
        let mut parent = ethrex_common::H256::zero();
        for n in 0..=40u64 {
            let h = header_with_ts(n, n * 100, parent);
            let hash = h.hash();
            store2
                .add_block(Block {
                    header: h,
                    body: BlockBody::default(),
                })
                .await
                .unwrap();
            store2.set_canonical_block_for_test(n, hash).await.unwrap();
            parent = hash;
        }
        store2
            .set_finalized_block_number_for_test(40)
            .await
            .unwrap();
        store2.set_latest_block_number_for_test(40).await.unwrap();

        // keep_recent = 25 → prune_ceiling = 15; persisted = 30. Now the margin wins.
        let pruner2 = HistoryPruner {
            store: store2.clone(),
            retention: Duration::from_secs(1),
            keep_recent: 25,
            persisted_floor_override: Some(30),
        };
        assert_eq!(
            pruner2.tick(10_000).await.unwrap(),
            15,
            "near-head margin wins"
        );
        assert_eq!(store2.get_earliest_block_number().await.unwrap(), 16);
    }

    /// A pass spanning more heights than `PRUNE_CHUNK_SIZE` must commit several
    /// chunks and land the pointer exactly at `target + 1`.
    ///
    /// Every other test prunes a range far smaller than the chunk size, so the loop
    /// body runs exactly once and the chunk arithmetic (`earliest += chunk`, the
    /// `remaining_target` clamp) is never exercised across iterations. This is also
    /// what backs the per-chunk pointer advance that lets a deadline-truncated pass
    /// resume where it stopped.
    #[tokio::test]
    async fn tick_spans_multiple_chunks() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        store.advance_earliest_block_number(0).await.unwrap();

        // Two-and-a-bit chunks' worth of prunable heights.
        let target = PRUNE_CHUNK_SIZE as u64 * 2 + 30;
        let head = target + 1;
        let mut parent = ethrex_common::H256::zero();
        for n in 0..=head {
            let h = header_with_ts(n, n * 100, parent);
            let hash = h.hash();
            store
                .add_block(Block {
                    header: h,
                    body: BlockBody::default(),
                })
                .await
                .unwrap();
            store.set_canonical_block_for_test(n, hash).await.unwrap();
            parent = hash;
        }
        store
            .set_finalized_block_number_for_test(target)
            .await
            .unwrap();
        store.set_latest_block_number_for_test(head).await.unwrap();

        let pruner = pruner_with_keep_recent(store.clone(), 1, 0);
        let pruned = pruner.tick(10_000_000).await.unwrap();

        // Heights 1..=target, genesis retained.
        assert_eq!(pruned as u64, target);
        assert!(
            pruned > PRUNE_CHUNK_SIZE,
            "this test is only meaningful if it spans more than one chunk"
        );
        assert_eq!(store.get_earliest_block_number().await.unwrap(), target + 1);
        for n in 1..=target {
            assert!(store.get_block_body(n).await.unwrap().is_none(), "body {n}");
        }
        assert!(
            store.get_block_body(head).await.unwrap().is_some(),
            "the head's body must survive"
        );
    }

    #[tokio::test]
    async fn tick_prunes_old_blocks() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        store.advance_earliest_block_number(0).await.unwrap();

        // ts 0..900 step 100; now=950, retention=200s -> cutoff ts<=750 -> block 7.
        let mut parent = ethrex_common::H256::zero();
        for n in 0..10u64 {
            let h = header_with_ts(n, n * 100, parent);
            let hash = h.hash();
            let block = Block {
                header: h,
                body: BlockBody::default(),
            };
            store.add_block(block).await.unwrap();
            store.set_canonical_block_for_test(n, hash).await.unwrap();
            parent = hash;
        }
        store.set_finalized_block_number_for_test(9).await.unwrap();
        store.set_latest_block_number_for_test(9).await.unwrap();

        // keep_recent = 0 isolates the retention/finalized logic from the head
        // floor (covered by its own tests below).
        let pruner = pruner_with_keep_recent(store.clone(), 200, 0);
        let pruned = pruner.tick(950).await.unwrap();

        // Heights 1..=7: genesis is never pruned, so 7 rather than 8. The pointer
        // still lands on 8 — `prune_block_heights` advances past the range it wrote.
        assert_eq!(pruned, 7);
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 8);

        for n in 0..10 {
            assert!(store.get_block_header(n).unwrap().is_some(), "header {n}");
        }
        assert!(
            store.get_block_body(0).await.unwrap().is_some(),
            "genesis body is retained"
        );
        for n in 1..=7 {
            assert!(store.get_block_body(n).await.unwrap().is_none(), "body {n}");
        }
        for n in 8..=9 {
            assert!(store.get_block_body(n).await.unwrap().is_some(), "body {n}");
        }
    }

    /// End-to-end: synthetic chain with an orphan, retention-driven pruning,
    /// restart no-op, then time-advanced second pass.
    #[tokio::test]
    async fn full_pruning_cycle_with_orphan_and_restart() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        store.advance_earliest_block_number(0).await.unwrap();

        // Canonical chain 0..=20, timestamps 0..2000 step 100.
        let mut parent = ethrex_common::H256::zero();
        for n in 0..=20u64 {
            let h = header_with_ts(n, n * 100, parent);
            let hash = h.hash();
            let block = Block {
                header: h,
                body: BlockBody::default(),
            };
            store.add_block(block).await.unwrap();
            store.set_canonical_block_for_test(n, hash).await.unwrap();
            parent = hash;
        }

        // Orphan at height 5 (different parent → distinct hash).
        let orphan = header_with_ts(5, 510, ethrex_common::H256::zero());
        let orphan_hash = orphan.hash();
        store.add_block_headers(vec![orphan]).await.unwrap();
        store
            .add_block_body(orphan_hash, BlockBody::default())
            .await
            .unwrap();

        store.set_finalized_block_number_for_test(20).await.unwrap();
        store.set_latest_block_number_for_test(20).await.unwrap();

        // Pass 1: now=1500, retention=500s → prune 1..=10 (genesis retained).
        // keep_recent = 0 isolates retention logic from the head floor.
        let pruner = pruner_with_keep_recent(store.clone(), 500, 0);
        let pruned = pruner.tick(1500).await.unwrap();
        assert_eq!(pruned, 10);

        for n in 0..=20 {
            assert!(store.get_block_header(n).unwrap().is_some(), "header {n}");
        }
        assert!(
            store.get_block_body(0).await.unwrap().is_some(),
            "genesis body is retained"
        );
        for n in 1..=10 {
            assert!(store.get_block_body(n).await.unwrap().is_none(), "body {n}");
        }
        for n in 11..=20 {
            assert!(store.get_block_body(n).await.unwrap().is_some(), "body {n}");
        }
        assert!(
            store
                .get_block_header_by_hash(orphan_hash)
                .unwrap()
                .is_none(),
            "orphan header should be deleted"
        );
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 11);

        // Pass 2: restart resilience — same now, no-op.
        let pruner2 = pruner_with_keep_recent(store.clone(), 500, 0);
        assert_eq!(pruner2.tick(1500).await.unwrap(), 0);
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 11);

        // Pass 3: now=2500 → prune 11..=20.
        let pruner3 = pruner_with_keep_recent(store.clone(), 500, 0);
        assert_eq!(pruner3.tick(2500).await.unwrap(), 10);
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 21);
        for n in 11..=20 {
            assert!(store.get_block_body(n).await.unwrap().is_none(), "body {n}");
        }
    }

    /// The head floor caps `target` at `head - keep_recent`, so the most
    /// recent `keep_recent` blocks (the state-regeneration window, including
    /// the head itself) keep their bodies even when finality and the retention
    /// window both reach the head.
    #[tokio::test]
    async fn tick_keeps_recent_head_window() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        store.advance_earliest_block_number(0).await.unwrap();

        // Chain 0..=20, all timestamps well below the cutoff so retention
        // alone would prune the whole chain.
        let mut parent = ethrex_common::H256::zero();
        for n in 0..=20u64 {
            let h = header_with_ts(n, n * 100, parent);
            let hash = h.hash();
            store
                .add_block(Block {
                    header: h,
                    body: BlockBody::default(),
                })
                .await
                .unwrap();
            store.set_canonical_block_for_test(n, hash).await.unwrap();
            parent = hash;
        }
        store.set_finalized_block_number_for_test(20).await.unwrap();
        store.set_latest_block_number_for_test(20).await.unwrap();

        // keep_recent = 5 → prune_ceiling = 15. now=10_000, retention=1s makes
        // every block "old", so target = min(finalized=20, retention=20, 15) = 15.
        let pruner = pruner_with_keep_recent(store.clone(), 1, 5);
        let pruned = pruner.tick(10_000).await.unwrap();

        assert_eq!(pruned, 15, "should prune heights 1..=15 (genesis retained)");
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 16);

        for n in 1..=15 {
            assert!(store.get_block_body(n).await.unwrap().is_none(), "body {n}");
        }
        // The head window (16..=20, incl. the head) keeps its bodies.
        for n in 16..=20 {
            assert!(store.get_block_body(n).await.unwrap().is_some(), "body {n}");
        }
    }

    /// Regression for the batch-sync brick: the pruner must never delete a
    /// body that `regenerate_head_state` re-executes on restart, i.e. nothing
    /// at or above the last *persisted* state block. During bulk/batch sync
    /// the persisted state can lag the head by thousands of blocks — far
    /// beyond `keep_recent` — so the persisted-state floor, not `keep_recent`,
    /// is what bounds the target.
    #[tokio::test]
    async fn tick_keeps_unpersisted_state_window() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        store.advance_earliest_block_number(0).await.unwrap();

        // Chain 0..=20, all timestamps well below the cutoff so retention,
        // finality, and the head margin would all otherwise reach the head.
        let mut parent = ethrex_common::H256::zero();
        for n in 0..=20u64 {
            let h = header_with_ts(n, n * 100, parent);
            let hash = h.hash();
            store
                .add_block(Block {
                    header: h,
                    body: BlockBody::default(),
                })
                .await
                .unwrap();
            store.set_canonical_block_for_test(n, hash).await.unwrap();
            parent = hash;
        }
        store.set_finalized_block_number_for_test(20).await.unwrap();
        store.set_latest_block_number_for_test(20).await.unwrap();

        // keep_recent = 0 (near-head margin disabled) and retention=1s makes
        // every block "old", so finalized/retention/prune_ceiling all reach 20.
        // But persisted state only reaches block 8 (simulating a large
        // unpersisted bulk-sync window). The pruner must stop at 8.
        let pruner = HistoryPruner {
            store: store.clone(),
            retention: Duration::from_secs(1),
            keep_recent: 0,
            persisted_floor_override: Some(8),
        };
        let pruned = pruner.tick(10_000).await.unwrap();

        assert_eq!(pruned, 8, "should prune heights 1..=8 only");
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 9);

        for n in 1..=8 {
            assert!(store.get_block_body(n).await.unwrap().is_none(), "body {n}");
        }
        // The unpersisted window (9..=20) keeps its bodies — regeneration needs
        // them on the next boot.
        for n in 9..=20 {
            assert!(store.get_block_body(n).await.unwrap().is_some(), "body {n}");
        }
    }

    /// Regression: on a chain shorter than `keep_recent` (e.g. a fresh node
    /// whose head is older than the retention window), the pruner must not
    /// touch the head's body — doing so stalls block production and bricks the
    /// node's state regeneration on restart.
    #[tokio::test]
    async fn tick_does_not_prune_head_when_chain_shorter_than_keep_recent() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        store.advance_earliest_block_number(0).await.unwrap();

        let mut parent = ethrex_common::H256::zero();
        for n in 0..=10u64 {
            let h = header_with_ts(n, n * 100, parent);
            let hash = h.hash();
            store
                .add_block(Block {
                    header: h,
                    body: BlockBody::default(),
                })
                .await
                .unwrap();
            store.set_canonical_block_for_test(n, hash).await.unwrap();
            parent = hash;
        }
        // Dev-style: finality tracks the head, and every block is older than
        // the retention window.
        store.set_finalized_block_number_for_test(10).await.unwrap();
        store.set_latest_block_number_for_test(10).await.unwrap();

        // keep_recent = 20 > head (10): prune_ceiling underflows → no-op pass.
        let pruner = pruner_with_keep_recent(store.clone(), 1, 20);
        let pruned = pruner.tick(10_000).await.unwrap();

        assert_eq!(pruned, 0, "nothing safe to prune yet");
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 0);
        for n in 0..=10 {
            assert!(
                store.get_block_body(n).await.unwrap().is_some(),
                "body {n} (incl. head) must survive"
            );
        }
    }
}
