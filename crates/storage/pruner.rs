//! History pruner task: deletes block bodies, receipts, transaction
//! locations, and non-canonical block data for heights older than the
//! configured retention window.

use crate::store::Store;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(feature = "metrics")]
use ethrex_metrics::pruning::METRICS_PRUNING;

const PRUNE_INTERVAL_SECS: u64 = 12;
const PRUNE_PASS_TIMEOUT_MS: u64 = 2_000;
// Heights deleted per pass. The key count scales with tx density: at ~70
// tx/block (Ethereum mainnet) a full pass deletes ~600K keys (4096 × (1 body +
// receipts + tx_locations)); on denser chains like BSC (~200 tx/block) it's
// ~1.6M. Larger batches outrun RocksDB compaction and trigger write stalls;
// smaller ones can fall behind block sync. 4096 is the largest value we've
// seen stay healthy on BSC mainnet.
const PRUNE_BATCH_SIZE: usize = 4_096;
// Heights per `prune_block_heights` call within a pass. Each call holds the
// decoded bodies for its whole chunk in memory from the gather phase until the
// write batch commits, so this bounds peak heap (4096 at once would reach
// hundreds of MB to GBs on large-block chains) and gives the pass deadline a
// chunk boundary to fire at.
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
// (`regenerate_head_state`), and that window can be far larger than 256 during
// bulk/batch sync — up to `BATCH_COMMIT_THRESHOLD × EXECUTE_BATCH_SIZE` blocks,
// where `EXECUTE_BATCH_SIZE` is operator-configurable. `tick` caps the prune
// target at the actual persisted height (`Store::get_latest_persisted_state_block`)
// for that; `KEEP_RECENT` is only the additional near-head margin layered on top.
const KEEP_RECENT: u64 = 256;

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

    /// Run forever. Every PRUNE_INTERVAL_SECS, run one pass. Errors are
    /// logged at ERROR level and don't stop the loop.
    pub async fn run(self) {
        let mut interval = tokio::time::interval(Duration::from_secs(PRUNE_INTERVAL_SECS));
        loop {
            interval.tick().await;
            if let Err(e) = self.tick(now_seconds()).await {
                tracing::error!(error = ?e, "history pruner pass failed");
            }
        }
    }

    /// Run one pass. Returns the number of heights processed.
    /// Public for testability (lets tests inject `now`).
    pub async fn tick(&self, now_secs: u64) -> Result<usize, crate::error::StoreError> {
        // Empty / pre-init store: nothing to prune. Bail before touching any
        // downstream reads so we don't surface MissingEarliestBlockNumber from
        // `find_canonical_block_by_timestamp`.
        let mut earliest = match self.store.get_earliest_block_number().await {
            Ok(n) => n,
            Err(crate::error::StoreError::MissingEarliestBlockNumber) => return Ok(0),
            Err(e) => return Err(e),
        };

        // Never prune within `keep_recent` of the head: the state DB persists
        // ~128 blocks behind the head and re-executes the rest from their
        // bodies on restart, so pruning inside that window bricks the node and
        // pruning the head's body stalls block production. If the chain is
        // shorter than `keep_recent`, there is nothing safe to prune yet.
        let head = match self.store.get_latest_block_number().await {
            Ok(n) => n,
            Err(crate::error::StoreError::MissingLatestBlockNumber) => return Ok(0),
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
        // above that point from its body. The unpersisted window is
        // ~DB_COMMIT_THRESHOLD blocks when live-following, but up to
        // BATCH_COMMIT_THRESHOLD × EXECUTE_BATCH_SIZE (thousands) during bulk
        // sync — so the fixed `KEEP_RECENT` margin is not sufficient on its own
        // and we cap at the actual persisted height.
        let persisted = match self.persisted_floor_override {
            Some(p) => p,
            None => self.store.get_latest_persisted_state_block().await?,
        };

        // Cap by every floor: finality, the retention window, the near-head
        // margin, and the persisted-state boundary.
        let target = finalized
            .min(retention_block)
            .min(prune_ceiling)
            .min(persisted);
        if earliest > target {
            return Ok(0);
        }

        #[cfg(feature = "metrics")]
        {
            METRICS_PRUNING.prune_target_block.set(target as i64);
            METRICS_PRUNING
                .prune_lag_blocks
                .set(target.saturating_sub(earliest) as i64);
        }
        let start = Instant::now();

        let deadline = start + Duration::from_millis(PRUNE_PASS_TIMEOUT_MS);
        let mut processed: usize = 0;

        // One PRUNE_CHUNK_SIZE chunk per loop iteration: gather fans out
        // across rayon threads, then a single write txn commits that chunk's
        // deletes (including its EarliestBlockNumber advance, so a pass cut
        // short by the deadline resumes where it stopped). The pass ends when
        // the target is reached, PRUNE_BATCH_SIZE heights have been
        // processed, or the deadline elapses between chunks.
        while earliest <= target && processed < PRUNE_BATCH_SIZE && Instant::now() < deadline {
            let remaining_budget = PRUNE_BATCH_SIZE - processed;
            let remaining_target = (target + 1 - earliest) as usize;
            let chunk = PRUNE_CHUNK_SIZE.min(remaining_budget).min(remaining_target);
            if chunk == 0 {
                break;
            }
            let _counts = self
                .store
                .prune_block_heights(earliest, chunk)
                .await
                .map_err(|e| {
                    tracing::error!(error = ?e, start = earliest, chunk, "prune_block_heights failed");
                    e
                })?;
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

    #[tokio::test]
    async fn tick_no_finalized_no_work() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        let pruner = HistoryPruner::new(store, Duration::from_secs(3600));
        let done = pruner.tick(10_000).await.unwrap();
        assert_eq!(done, 0);
    }

    #[tokio::test]
    async fn tick_prunes_old_blocks() {
        let store = Store::new("", EngineType::InMemory).unwrap();
        store.update_earliest_block_number(0).await.unwrap();

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

        assert_eq!(pruned, 8);
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 8);

        for n in 0..10 {
            assert!(store.get_block_header(n).unwrap().is_some(), "header {n}");
        }
        for n in 0..=7 {
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
        store.update_earliest_block_number(0).await.unwrap();

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

        // Pass 1: now=1500, retention=500s → prune 0..=10.
        // keep_recent = 0 isolates retention logic from the head floor.
        let pruner = pruner_with_keep_recent(store.clone(), 500, 0);
        let pruned = pruner.tick(1500).await.unwrap();
        assert_eq!(pruned, 11);

        for n in 0..=20 {
            assert!(store.get_block_header(n).unwrap().is_some(), "header {n}");
        }
        for n in 0..=10 {
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
        store.update_earliest_block_number(0).await.unwrap();

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

        assert_eq!(pruned, 16, "should prune heights 0..=15");
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 16);

        for n in 0..=15 {
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
        store.update_earliest_block_number(0).await.unwrap();

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

        assert_eq!(pruned, 9, "should prune heights 0..=8 only");
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 9);

        for n in 0..=8 {
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
        store.update_earliest_block_number(0).await.unwrap();

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
