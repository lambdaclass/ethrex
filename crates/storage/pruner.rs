//! History pruner task: deletes block bodies, receipts, transaction
//! locations, and non-canonical block data for heights older than the
//! configured retention window.
//!
//! See docs/superpowers/specs/2026-05-12-history-pruning-design.md.

use crate::store::Store;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(feature = "metrics")]
use ethrex_metrics::pruning::METRICS_PRUNING;

const PRUNE_INTERVAL_SECS: u64 = 12;
const PRUNE_PASS_TIMEOUT_MS: u64 = 2_000;
// Sized for post-pivot heights: a single pass deletes ~600K keys
// (~4096 bodies + receipts + tx_locations for tx-heavy chains). Larger
// batches outrun RocksDB compaction and trigger write stalls; smaller
// ones can fall behind block sync. 4096 is the largest value we've seen
// stay healthy on BSC mainnet.
const PRUNE_BATCH_SIZE: usize = 4_096;
// Used as the prune floor when no `FinalizedBlockNumber` is set (chains
// without engine-API finality, or a node before its first FCU). Covers
// reorg depths far beyond mainnet norms while letting the pruner do
// useful work.
const SAFETY_DISTANCE: u64 = 256;

pub struct HistoryPruner {
    store: Store,
    retention: Duration,
}

impl HistoryPruner {
    pub fn new(store: Store, retention: Duration) -> Self {
        Self { store, retention }
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

        // Prefer FinalizedBlockNumber as the prune floor. Chains without
        // engine-API finality (e.g. BSC PoSA) never write it; fall back to
        // `head - SAFETY_DISTANCE` so the pruner can still do useful work.
        let finalized = match self.store.get_finalized_block_number().await? {
            Some(n) => n,
            None => match self.store.get_latest_block_number().await {
                Ok(head) => head.saturating_sub(SAFETY_DISTANCE),
                Err(crate::error::StoreError::MissingLatestBlockNumber) => return Ok(0),
                Err(e) => return Err(e),
            },
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

        let target = finalized.min(retention_block);
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

        // One parallel batch per loop iteration: gather phase fans out across
        // rayon threads, then a single write txn commits all deletes for that
        // chunk. Loop is still capped by PRUNE_BATCH_SIZE and the per-pass
        // deadline.
        while earliest <= target && processed < PRUNE_BATCH_SIZE && Instant::now() < deadline {
            let remaining_budget = PRUNE_BATCH_SIZE - processed;
            let remaining_target = (target + 1 - earliest) as usize;
            let chunk = remaining_budget.min(remaining_target);
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

        let pruner = HistoryPruner::new(store.clone(), Duration::from_secs(200));
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

        // Pass 1: now=1500, retention=500s → prune 0..=10.
        let pruner = HistoryPruner::new(store.clone(), Duration::from_secs(500));
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
        let pruner2 = HistoryPruner::new(store.clone(), Duration::from_secs(500));
        assert_eq!(pruner2.tick(1500).await.unwrap(), 0);
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 11);

        // Pass 3: now=2500 → prune 11..=20.
        let pruner3 = HistoryPruner::new(store.clone(), Duration::from_secs(500));
        assert_eq!(pruner3.tick(2500).await.unwrap(), 10);
        assert_eq!(store.get_earliest_block_number().await.unwrap(), 21);
        for n in 11..=20 {
            assert!(store.get_block_body(n).await.unwrap().is_none(), "body {n}");
        }
    }
}
