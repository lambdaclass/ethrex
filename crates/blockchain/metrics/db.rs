use prometheus::{IntGauge, IntGaugeVec, register_int_gauge, register_int_gauge_vec};
use std::sync::LazyLock;

// Metrics defined in this module register into the Prometheus default registry,
// so the metrics API exposes them via `gather_default_metrics()`. They are
// populated by a periodic collector that reads `Store::rocksdb_stats()`.

pub static METRICS_DB: LazyLock<MetricsDB> = LazyLock::new(MetricsDB::default);

/// RocksDB observability metrics: per-column-family sizes/keys/files plus
/// DB-wide block-cache and compaction counters, for granular DB visibility.
#[derive(Debug, Clone)]
pub struct MetricsDB {
    // --- Per-column-family (label: `cf`) ---
    cf_size_bytes: IntGaugeVec,
    cf_total_sst_bytes: IntGaugeVec,
    cf_live_data_bytes: IntGaugeVec,
    cf_num_keys: IntGaugeVec,
    cf_num_files: IntGaugeVec,
    cf_blob_bytes: IntGaugeVec,
    cf_pending_compaction_bytes: IntGaugeVec,
    cf_memtable_bytes: IntGaugeVec,

    // --- DB-wide ---
    total_live_sst_bytes: IntGauge,
    block_cache_usage_bytes: IntGauge,
    block_cache_capacity_bytes: IntGauge,
    block_cache_pinned_bytes: IntGauge,
    running_compactions: IntGauge,
    /// Cumulative block-cache hits (0 unless RocksDB statistics are enabled).
    block_cache_hits: IntGauge,
    /// Cumulative block-cache misses (0 unless RocksDB statistics are enabled).
    block_cache_misses: IntGauge,
    /// Lowest block with full chain data on disk (the history-backfill frontier /
    /// `earliest_block_number`); descends toward the floor as backfill runs.
    backfill_frontier_block: IntGauge,
}

impl Default for MetricsDB {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsDB {
    pub fn new() -> Self {
        MetricsDB {
            cf_size_bytes: register_int_gauge_vec!(
                "ethrex_db_cf_size_bytes",
                "Live SST bytes on disk per column family",
                &["cf"]
            )
            .expect("Failed to create ethrex_db_cf_size_bytes"),
            cf_total_sst_bytes: register_int_gauge_vec!(
                "ethrex_db_cf_total_sst_bytes",
                "Total SST bytes per CF including not-yet-compacted (for space amplification)",
                &["cf"]
            )
            .expect("Failed to create ethrex_db_cf_total_sst_bytes"),
            cf_live_data_bytes: register_int_gauge_vec!(
                "ethrex_db_cf_live_data_bytes",
                "Estimated live (logical) data bytes per column family",
                &["cf"]
            )
            .expect("Failed to create ethrex_db_cf_live_data_bytes"),
            cf_num_keys: register_int_gauge_vec!(
                "ethrex_db_cf_num_keys",
                "Estimated number of keys per column family",
                &["cf"]
            )
            .expect("Failed to create ethrex_db_cf_num_keys"),
            cf_num_files: register_int_gauge_vec!(
                "ethrex_db_cf_num_files",
                "Live SST file count per column family",
                &["cf"]
            )
            .expect("Failed to create ethrex_db_cf_num_files"),
            cf_blob_bytes: register_int_gauge_vec!(
                "ethrex_db_cf_blob_bytes",
                "Live blob file bytes per column family (account_codes uses blobs)",
                &["cf"]
            )
            .expect("Failed to create ethrex_db_cf_blob_bytes"),
            cf_pending_compaction_bytes: register_int_gauge_vec!(
                "ethrex_db_cf_pending_compaction_bytes",
                "Estimated pending compaction bytes per column family (write-debt)",
                &["cf"]
            )
            .expect("Failed to create ethrex_db_cf_pending_compaction_bytes"),
            cf_memtable_bytes: register_int_gauge_vec!(
                "ethrex_db_cf_memtable_bytes",
                "Current memtable bytes per column family",
                &["cf"]
            )
            .expect("Failed to create ethrex_db_cf_memtable_bytes"),
            total_live_sst_bytes: register_int_gauge!(
                "ethrex_db_total_live_sst_bytes",
                "Total live SST bytes on disk across all column families"
            )
            .expect("Failed to create ethrex_db_total_live_sst_bytes"),
            block_cache_usage_bytes: register_int_gauge!(
                "ethrex_db_block_cache_usage_bytes",
                "RocksDB shared block cache bytes in use"
            )
            .expect("Failed to create ethrex_db_block_cache_usage_bytes"),
            block_cache_capacity_bytes: register_int_gauge!(
                "ethrex_db_block_cache_capacity_bytes",
                "RocksDB shared block cache capacity in bytes"
            )
            .expect("Failed to create ethrex_db_block_cache_capacity_bytes"),
            block_cache_pinned_bytes: register_int_gauge!(
                "ethrex_db_block_cache_pinned_bytes",
                "RocksDB block cache bytes pinned (index/filter blocks)"
            )
            .expect("Failed to create ethrex_db_block_cache_pinned_bytes"),
            running_compactions: register_int_gauge!(
                "ethrex_db_running_compactions",
                "Number of currently running RocksDB compactions"
            )
            .expect("Failed to create ethrex_db_running_compactions"),
            block_cache_hits: register_int_gauge!(
                "ethrex_db_block_cache_hits_total",
                "Cumulative RocksDB block cache hits (requires statistics enabled)"
            )
            .expect("Failed to create ethrex_db_block_cache_hits_total"),
            block_cache_misses: register_int_gauge!(
                "ethrex_db_block_cache_misses_total",
                "Cumulative RocksDB block cache misses (requires statistics enabled)"
            )
            .expect("Failed to create ethrex_db_block_cache_misses_total"),
            backfill_frontier_block: register_int_gauge!(
                "ethrex_db_backfill_frontier_block",
                "Lowest block with full chain data on disk (history-backfill frontier)"
            )
            .expect("Failed to create ethrex_db_backfill_frontier_block"),
        }
    }

    /// Set the per-CF gauges for one column family.
    #[allow(clippy::too_many_arguments)]
    pub fn set_cf(
        &self,
        cf: &str,
        live_sst_bytes: u64,
        total_sst_bytes: u64,
        live_data_bytes: u64,
        num_keys: u64,
        num_files: u64,
        blob_bytes: u64,
        pending_compaction_bytes: u64,
        memtable_bytes: u64,
    ) {
        let l = &[cf];
        self.cf_size_bytes
            .with_label_values(l)
            .set(live_sst_bytes as i64);
        self.cf_total_sst_bytes
            .with_label_values(l)
            .set(total_sst_bytes as i64);
        self.cf_live_data_bytes
            .with_label_values(l)
            .set(live_data_bytes as i64);
        self.cf_num_keys.with_label_values(l).set(num_keys as i64);
        self.cf_num_files.with_label_values(l).set(num_files as i64);
        self.cf_blob_bytes
            .with_label_values(l)
            .set(blob_bytes as i64);
        self.cf_pending_compaction_bytes
            .with_label_values(l)
            .set(pending_compaction_bytes as i64);
        self.cf_memtable_bytes
            .with_label_values(l)
            .set(memtable_bytes as i64);
    }

    /// Set the DB-wide gauges.
    #[allow(clippy::too_many_arguments)]
    pub fn set_global(
        &self,
        total_live_sst_bytes: u64,
        block_cache_usage_bytes: u64,
        block_cache_capacity_bytes: u64,
        block_cache_pinned_bytes: u64,
        running_compactions: u64,
        block_cache_hits: u64,
        block_cache_misses: u64,
    ) {
        self.total_live_sst_bytes.set(total_live_sst_bytes as i64);
        self.block_cache_usage_bytes
            .set(block_cache_usage_bytes as i64);
        self.block_cache_capacity_bytes
            .set(block_cache_capacity_bytes as i64);
        self.block_cache_pinned_bytes
            .set(block_cache_pinned_bytes as i64);
        self.running_compactions.set(running_compactions as i64);
        self.block_cache_hits.set(block_cache_hits as i64);
        self.block_cache_misses.set(block_cache_misses as i64);
    }

    /// Set the backfill frontier (lowest block with full chain data).
    pub fn set_backfill_frontier(&self, block: u64) {
        self.backfill_frontier_block.set(block as i64);
    }
}
