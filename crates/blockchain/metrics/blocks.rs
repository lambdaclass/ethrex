use prometheus::Histogram;
use prometheus::HistogramOpts;
use prometheus::{Encoder, Gauge, IntGauge, Registry, TextEncoder};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_BLOCKS: LazyLock<MetricsBlocks> = LazyLock::new(MetricsBlocks::default);

#[derive(Debug, Clone)]
pub struct MetricsBlocks {
    gas_limit: Gauge,
    /// Keeps track of the block number of the last processed block
    block_number: IntGauge,
    gigagas: Gauge,
    gigagas_histogram: Histogram,
    gigagas_block_building: Gauge,
    block_building_ms: IntGauge,
    block_building_base_fee: IntGauge,
    gas_used: Gauge,
    transaction_count: IntGauge,
    execution_ms: IntGauge,
    merkle_ms: IntGauge,
    store_ms: IntGauge,
    /// Keeps track of the head block number
    head_height: IntGauge,
    // Pipeline-specific metrics
    /// Block validation time in milliseconds
    validate_ms: IntGauge,
    /// Time spent on merkle operations concurrent with execution
    merkle_concurrent_ms: IntGauge,
    /// Time spent draining merkle queue after execution completes
    merkle_drain_ms: IntGauge,
    /// Percentage of merkle work done concurrently with execution
    merkle_overlap_pct: IntGauge,
    /// Total warmer thread execution time in milliseconds
    warmer_ms: IntGauge,
    /// Warmer finished early (positive) or late (negative) relative to exec, in ms
    warmer_early_ms: IntGauge,
    // Flight recorder Phase 2: store sub-phases
    /// Time writing blocks, receipts, and code to the write batch
    store_db_write_ms: IntGauge,
    /// Time waiting for trie layer to become ready
    store_trie_wait_ms: IntGauge,
    /// Time for RocksDB commit
    store_db_commit_ms: IntGauge,
    // Flight recorder Phase 2: RPC + e2e metrics
    /// Full e2e latency including RPC overhead
    e2e_ms: IntGauge,
    /// Channel handoff latency
    channel_handoff_ms: IntGauge,
    /// Block construction time in RPC layer
    rpc_block_construction_ms: IntGauge,
    /// Mgas/s throughput per block (pipeline-only, no RPC overhead)
    mgas_per_second: Gauge,
}

impl Default for MetricsBlocks {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsBlocks {
    pub fn new() -> Self {
        MetricsBlocks {
            gas_limit: Gauge::new(
                "gas_limit",
                "Keeps track of the percentage of gas limit used by the last processed block",
            )
            .expect("Failed to create gas_limit metric"),
            block_number: IntGauge::new(
                "block_number",
                "Keeps track of the block number for the last processed block",
            )
            .expect("Failed to create block_number metric"),
            gigagas: Gauge::new(
                "gigagas",
                "Keeps track of the block execution throughput through gigagas/s",
            )
            .expect("Failed to create gigagas metric"),
            gigagas_histogram: Histogram::with_opts(
                HistogramOpts::new(
                    "gigagas_histogram",
                    "Histogram of the block execution throughput through gigagas/s",
                )
                .buckets({
                    let mut buckets = vec![0.0];
                    // 0.0 is added separately; next 5 buckets cover 0.03 to 0.15 Ggas (30 Mgas resolution)
                    buckets.extend(prometheus::linear_buckets(0.03, 0.03, 5).expect("Invalid bucket params"));
                    // 0.16 to 1.5 Ggas (10 Mgas resolution) -- 0.15 is covered by the previous bucket range
                    buckets.extend(prometheus::linear_buckets(0.16, 0.01, 135).expect("Invalid bucket params"));
                    // 1.6 to 2.0 Ggas (100 Mgas resolution)
                    buckets.extend(prometheus::linear_buckets(1.6, 0.1, 5).expect("Invalid bucket params"));
                    // High values
                    buckets.extend(vec![2.5, 3.0, 4.0, 5.0, 10.0, 20.0]);
                    buckets
                }),
            )
            .expect("Failed to create gigagas_histogram metric"),
            gigagas_block_building: Gauge::new(
                "gigagas_block_building",
                "Keeps track of the block building throughput through gigagas/s",
            )
            .expect("Failed to create gigagas_block_building metric"),
            block_building_ms: IntGauge::new(
                "block_building_ms",
                "Keeps track of the block building throughput through miliseconds",
            )
            .expect("Failed to create block_building_ms metric"),
            block_building_base_fee: IntGauge::new(
                "block_building_base_fee",
                "Keeps track of the block building base fee",
            )
            .expect("Failed to create block_building_base_fee metric"),
            gas_used: Gauge::new(
                "gas_used",
                "Keeps track of the gas used in the last processed block",
            )
            .expect("Failed to create gas_used metric"),
            head_height: IntGauge::new(
                "head_height",
                "Keeps track of the block number for the head of the chain",
            )
            .expect("Failed to create head_height metric"),
            execution_ms: IntGauge::new(
                "execution_ms",
                "Keeps track of the execution time spent in block execution in miliseconds",
            )
            .expect("Failed to create execution_ms metric"),
            merkle_ms: IntGauge::new(
                "merkle_ms",
                "Keeps track of the execution time spent in block merkelization in miliseconds",
            )
            .expect("Failed to create merkle_ms metric"),
            store_ms: IntGauge::new(
                "store_ms",
                "Keeps track of the execution time spent in block storage in miliseconds",
            )
            .expect("Failed to create store_ms metric"),
            transaction_count: IntGauge::new(
                "transaction_count",
                "Keeps track of transaction count in a block",
            )
            .expect("Failed to create transaction_count metric"),
            validate_ms: IntGauge::new(
                "validate_ms",
                "Block validation time in milliseconds",
            )
            .expect("Failed to create validate_ms metric"),
            merkle_concurrent_ms: IntGauge::new(
                "merkle_concurrent_ms",
                "Time spent on merkle operations concurrent with execution in milliseconds",
            )
            .expect("Failed to create merkle_concurrent_ms metric"),
            merkle_drain_ms: IntGauge::new(
                "merkle_drain_ms",
                "Time spent draining merkle queue after execution completes in milliseconds",
            )
            .expect("Failed to create merkle_drain_ms metric"),
            merkle_overlap_pct: IntGauge::new(
                "merkle_overlap_pct",
                "Percentage of merkle work done concurrently with execution",
            )
            .expect("Failed to create merkle_overlap_pct metric"),
            warmer_ms: IntGauge::new(
                "warmer_ms",
                "Total warmer thread execution time in milliseconds",
            )
            .expect("Failed to create warmer_ms metric"),
            warmer_early_ms: IntGauge::new(
                "warmer_early_ms",
                "Warmer finished early (positive) or late (negative) relative to exec in milliseconds",
            )
            .expect("Failed to create warmer_early_ms metric"),
            store_db_write_ms: IntGauge::new(
                "store_db_write_ms",
                "Time writing blocks, receipts, and code to the write batch in milliseconds",
            )
            .expect("Failed to create store_db_write_ms metric"),
            store_trie_wait_ms: IntGauge::new(
                "store_trie_wait_ms",
                "Time waiting for trie layer to become ready in milliseconds",
            )
            .expect("Failed to create store_trie_wait_ms metric"),
            store_db_commit_ms: IntGauge::new(
                "store_db_commit_ms",
                "Time for RocksDB commit in milliseconds",
            )
            .expect("Failed to create store_db_commit_ms metric"),
            e2e_ms: IntGauge::new(
                "e2e_ms",
                "Full engine_newPayload e2e latency including RPC overhead in milliseconds",
            )
            .expect("Failed to create e2e_ms metric"),
            channel_handoff_ms: IntGauge::new(
                "channel_handoff_ms",
                "Channel handoff latency in milliseconds",
            )
            .expect("Failed to create channel_handoff_ms metric"),
            rpc_block_construction_ms: IntGauge::new(
                "rpc_block_construction_ms",
                "Block construction time in RPC layer in milliseconds",
            )
            .expect("Failed to create rpc_block_construction_ms metric"),
            mgas_per_second: Gauge::new(
                "mgas_per_second",
                "Pipeline throughput in Mgas/s per block",
            )
            .expect("Failed to create mgas_per_second metric"),
        }
    }

    pub fn set_transaction_count(&self, transaction_count: i64) {
        self.transaction_count.set(transaction_count);
    }

    pub fn set_execution_ms(&self, execution_ms: i64) {
        self.execution_ms.set(execution_ms);
    }

    pub fn set_merkle_ms(&self, merkle_ms: i64) {
        self.merkle_ms.set(merkle_ms);
    }

    pub fn set_store_ms(&self, store_ms: i64) {
        self.store_ms.set(store_ms);
    }

    pub fn set_latest_block_gas_limit(&self, gas_limit: f64) {
        self.gas_limit.set(gas_limit);
    }

    pub fn set_latest_gigagas(&self, gigagas: f64) {
        self.gigagas.set(gigagas);
        self.gigagas_histogram.observe(gigagas);
    }

    pub fn set_latest_gigagas_block_building(&self, gigagas: f64) {
        self.gigagas_block_building.set(gigagas);
    }

    pub fn set_block_building_ms(&self, ms: i64) {
        self.block_building_ms.set(ms);
    }

    pub fn set_block_building_base_fee(&self, base_fee: i64) {
        self.block_building_base_fee.set(base_fee);
    }

    pub fn set_block_number(&self, block_number: u64) {
        self.block_number.set(block_number.cast_signed());
    }

    pub fn set_head_height(&self, head_height: u64) {
        self.head_height.set(head_height.cast_signed());
    }

    pub fn set_latest_gas_used(&self, gas_used: f64) {
        self.gas_used.set(gas_used);
    }

    pub fn set_validate_ms(&self, validate_ms: i64) {
        self.validate_ms.set(validate_ms);
    }

    pub fn set_merkle_concurrent_ms(&self, merkle_concurrent_ms: i64) {
        self.merkle_concurrent_ms.set(merkle_concurrent_ms);
    }

    pub fn set_merkle_drain_ms(&self, merkle_drain_ms: i64) {
        self.merkle_drain_ms.set(merkle_drain_ms);
    }

    pub fn set_merkle_overlap_pct(&self, merkle_overlap_pct: i64) {
        self.merkle_overlap_pct.set(merkle_overlap_pct);
    }

    pub fn set_warmer_ms(&self, warmer_ms: i64) {
        self.warmer_ms.set(warmer_ms);
    }

    pub fn set_warmer_early_ms(&self, warmer_early_ms: i64) {
        self.warmer_early_ms.set(warmer_early_ms);
    }

    pub fn set_store_db_write_ms(&self, ms: i64) {
        self.store_db_write_ms.set(ms);
    }

    pub fn set_store_trie_wait_ms(&self, ms: i64) {
        self.store_trie_wait_ms.set(ms);
    }

    pub fn set_store_db_commit_ms(&self, ms: i64) {
        self.store_db_commit_ms.set(ms);
    }

    pub fn set_e2e_ms(&self, ms: i64) {
        self.e2e_ms.set(ms);
    }

    pub fn set_channel_handoff_ms(&self, ms: i64) {
        self.channel_handoff_ms.set(ms);
    }

    pub fn set_rpc_block_construction_ms(&self, ms: i64) {
        self.rpc_block_construction_ms.set(ms);
    }

    pub fn set_mgas_per_second(&self, mgas: f64) {
        self.mgas_per_second.set(mgas);
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        if self.block_number.get() <= 0 {
            return Ok(String::new());
        }

        let r = Registry::new();

        r.register(Box::new(self.gas_limit.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.block_number.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.gigagas.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.gigagas_histogram.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.gigagas_block_building.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.gas_used.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.block_building_base_fee.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.block_building_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.head_height.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.store_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.execution_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.merkle_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.transaction_count.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.validate_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.merkle_concurrent_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.merkle_drain_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.merkle_overlap_pct.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.warmer_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.warmer_early_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.store_db_write_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.store_trie_wait_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.store_db_commit_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.e2e_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.channel_handoff_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.rpc_block_construction_ms.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.mgas_per_second.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        let encoder = TextEncoder::new();
        let metric_families = r.gather();

        let mut buffer = Vec::new();
        encoder
            .encode(&metric_families, &mut buffer)
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        let res = String::from_utf8(buffer)?;

        Ok(res)
    }
}
