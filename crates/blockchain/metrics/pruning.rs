use prometheus::{
    Histogram, IntCounter, IntGauge, register_histogram, register_int_counter, register_int_gauge,
};
use std::sync::LazyLock;

pub static METRICS_PRUNING: LazyLock<MetricsPruning> = LazyLock::new(MetricsPruning::default);

#[derive(Debug, Clone)]
pub struct MetricsPruning {
    pub earliest_block_number: IntGauge,
    pub prune_target_block: IntGauge,
    pub prune_lag_blocks: IntGauge,
    pub pass_duration_ms: Histogram,
    pub pass_blocks: Histogram,
    pub bodies_deleted: IntCounter,
    pub receipts_deleted: IntCounter,
    pub tx_locations_deleted: IntCounter,
    pub orphan_headers_deleted: IntCounter,
    pub index_entries_deleted: IntCounter,
}

impl MetricsPruning {
    fn new() -> Self {
        Self {
            earliest_block_number: register_int_gauge!(
                "ethrex_pruning_earliest_block",
                "Lowest block whose body/receipts may still be in the DB"
            )
            .expect("Failed to create ethrex_pruning_earliest_block"),
            prune_target_block: register_int_gauge!(
                "ethrex_pruning_target_block",
                "Latest block the pruner is allowed to delete"
            )
            .expect("Failed to create ethrex_pruning_target_block"),
            prune_lag_blocks: register_int_gauge!(
                "ethrex_pruning_lag_blocks",
                "Blocks remaining between earliest_block and target"
            )
            .expect("Failed to create ethrex_pruning_lag_blocks"),
            pass_duration_ms: register_histogram!(
                "ethrex_pruning_pass_duration_ms",
                "Wall-clock duration of one prune pass in milliseconds"
            )
            .expect("Failed to create ethrex_pruning_pass_duration_ms"),
            pass_blocks: register_histogram!(
                "ethrex_pruning_pass_blocks",
                "Number of block heights processed in one pass"
            )
            .expect("Failed to create ethrex_pruning_pass_blocks"),
            bodies_deleted: register_int_counter!(
                "ethrex_pruning_bodies_deleted_total",
                "Total bodies deleted"
            )
            .expect("Failed to create ethrex_pruning_bodies_deleted_total"),
            receipts_deleted: register_int_counter!(
                "ethrex_pruning_receipts_deleted_total",
                "Total receipt rows deleted"
            )
            .expect("Failed to create ethrex_pruning_receipts_deleted_total"),
            tx_locations_deleted: register_int_counter!(
                "ethrex_pruning_tx_locations_deleted_total",
                "Total (transaction, pruned-block) location entries removed. Counts removals, \
                 not deleted CF rows: a tx in N pruned blocks counts N times, and a row whose \
                 location list still has surviving entries is rewritten (trimmed), not deleted"
            )
            .expect("Failed to create ethrex_pruning_tx_locations_deleted_total"),
            orphan_headers_deleted: register_int_counter!(
                "ethrex_pruning_orphan_headers_deleted_total",
                "Total non-canonical headers deleted"
            )
            .expect("Failed to create ethrex_pruning_orphan_headers_deleted_total"),
            index_entries_deleted: register_int_counter!(
                "ethrex_pruning_index_entries_deleted_total",
                "Total BLOCK_HASHES_BY_NUMBER entries deleted"
            )
            .expect("Failed to create ethrex_pruning_index_entries_deleted_total"),
        }
    }
}

impl Default for MetricsPruning {
    fn default() -> Self {
        Self::new()
    }
}
