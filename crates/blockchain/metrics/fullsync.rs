use prometheus::{Encoder, Gauge, IntCounter, IntGauge, Registry, TextEncoder};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_FULLSYNC: LazyLock<MetricsFullSync> = LazyLock::new(MetricsFullSync::default);

#[derive(Debug, Clone)]
pub struct MetricsFullSync {
    // Phase & progress
    stage: IntGauge,
    target_block: IntGauge,
    lowest_header: IntGauge,
    headers_downloaded: IntCounter,
    bodies_downloaded: IntCounter,
    blocks_executed: IntGauge,
    blocks_total: IntGauge,

    // Rates (in-process, instant)
    headers_per_second: Gauge,
    bodies_per_second: Gauge,
    blocks_per_second: Gauge,

    // Batch timing
    batch_body_download_ms: Gauge,
    batch_execution_ms: Gauge,
    batch_merkle_ms: Gauge,
    batch_store_ms: Gauge,
    batch_total_ms: Gauge,
    batch_size: IntGauge,

    // Timestamps (Unix epoch seconds, for Grafana elapsed time calculation)
    header_stage_start_timestamp: Gauge,
    execution_stage_start_timestamp: Gauge,

    // Reliability
    header_failures: IntCounter,
    body_failures: IntCounter,
    cycles_started: IntCounter,
    cycles_completed: IntCounter,
}

impl Default for MetricsFullSync {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsFullSync {
    pub fn new() -> Self {
        MetricsFullSync {
            stage: IntGauge::new(
                "fullsync_stage",
                "Current full sync stage: 0=idle, 1=downloading_headers, 2=downloading_bodies, 3=executing_blocks",
            )
            .expect("Failed to create fullsync_stage metric"),
            target_block: IntGauge::new(
                "fullsync_target_block",
                "Chain tip block number (sync target)",
            )
            .expect("Failed to create fullsync_target_block metric"),
            lowest_header: IntGauge::new(
                "fullsync_lowest_header",
                "Lowest block number whose header has been downloaded (decreases during header walk)",
            )
            .expect("Failed to create fullsync_lowest_header metric"),
            headers_downloaded: IntCounter::new(
                "fullsync_headers_downloaded",
                "Total headers downloaded",
            )
            .expect("Failed to create fullsync_headers_downloaded metric"),
            bodies_downloaded: IntCounter::new(
                "fullsync_bodies_downloaded",
                "Total bodies downloaded",
            )
            .expect("Failed to create fullsync_bodies_downloaded metric"),
            blocks_executed: IntGauge::new(
                "fullsync_blocks_executed",
                "Highest block number executed so far",
            )
            .expect("Failed to create fullsync_blocks_executed metric"),
            blocks_total: IntGauge::new(
                "fullsync_blocks_total",
                "Total blocks to execute in current cycle",
            )
            .expect("Failed to create fullsync_blocks_total metric"),

            // Rates
            headers_per_second: Gauge::new(
                "fullsync_headers_per_second",
                "Headers downloaded per second (averaged over last batch)",
            )
            .expect("Failed to create fullsync_headers_per_second metric"),
            bodies_per_second: Gauge::new(
                "fullsync_bodies_per_second",
                "Bodies downloaded per second (averaged over last batch)",
            )
            .expect("Failed to create fullsync_bodies_per_second metric"),
            blocks_per_second: Gauge::new(
                "fullsync_blocks_per_second",
                "Blocks executed per second (averaged over last batch)",
            )
            .expect("Failed to create fullsync_blocks_per_second metric"),

            // Batch timing
            batch_body_download_ms: Gauge::new(
                "fullsync_batch_body_download_ms",
                "Body download time for last batch in milliseconds",
            )
            .expect("Failed to create fullsync_batch_body_download_ms metric"),
            batch_execution_ms: Gauge::new(
                "fullsync_batch_execution_ms",
                "EVM execution time for last batch in milliseconds",
            )
            .expect("Failed to create fullsync_batch_execution_ms metric"),
            batch_merkle_ms: Gauge::new(
                "fullsync_batch_merkle_ms",
                "Merkleization time for last batch in milliseconds",
            )
            .expect("Failed to create fullsync_batch_merkle_ms metric"),
            batch_store_ms: Gauge::new(
                "fullsync_batch_store_ms",
                "Storage write time for last batch in milliseconds",
            )
            .expect("Failed to create fullsync_batch_store_ms metric"),
            batch_total_ms: Gauge::new(
                "fullsync_batch_total_ms",
                "Total time for last batch in milliseconds",
            )
            .expect("Failed to create fullsync_batch_total_ms metric"),
            batch_size: IntGauge::new(
                "fullsync_batch_size",
                "Number of blocks in last batch",
            )
            .expect("Failed to create fullsync_batch_size metric"),

            // Timestamps
            header_stage_start_timestamp: Gauge::new(
                "fullsync_header_stage_start_timestamp",
                "Unix timestamp (seconds) when header download stage began",
            )
            .expect("Failed to create fullsync_header_stage_start_timestamp metric"),
            execution_stage_start_timestamp: Gauge::new(
                "fullsync_execution_stage_start_timestamp",
                "Unix timestamp (seconds) when block execution stage began",
            )
            .expect("Failed to create fullsync_execution_stage_start_timestamp metric"),

            // Reliability
            header_failures: IntCounter::new(
                "fullsync_header_failures",
                "Total header fetch failures",
            )
            .expect("Failed to create fullsync_header_failures metric"),
            body_failures: IntCounter::new(
                "fullsync_body_failures",
                "Total body fetch failures",
            )
            .expect("Failed to create fullsync_body_failures metric"),
            cycles_started: IntCounter::new(
                "fullsync_cycles_started",
                "Number of sync cycles initiated",
            )
            .expect("Failed to create fullsync_cycles_started metric"),
            cycles_completed: IntCounter::new(
                "fullsync_cycles_completed",
                "Number of sync cycles that completed successfully",
            )
            .expect("Failed to create fullsync_cycles_completed metric"),
        }
    }

    // Phase & progress setters
    pub fn set_stage(&self, stage: i64) {
        self.stage.set(stage);
    }
    pub fn set_target_block(&self, block: u64) {
        self.target_block.set(block.cast_signed());
    }
    pub fn set_lowest_header(&self, block: u64) {
        self.lowest_header.set(block.cast_signed());
    }
    pub fn inc_headers_downloaded(&self, count: u64) {
        self.headers_downloaded.inc_by(count);
    }
    pub fn inc_bodies_downloaded(&self, count: u64) {
        self.bodies_downloaded.inc_by(count);
    }
    pub fn set_blocks_executed(&self, block: u64) {
        self.blocks_executed.set(block.cast_signed());
    }
    pub fn set_blocks_total(&self, total: u64) {
        self.blocks_total.set(total.cast_signed());
    }

    // Rate setters
    pub fn set_headers_per_second(&self, rate: f64) {
        self.headers_per_second.set(rate);
    }
    pub fn set_bodies_per_second(&self, rate: f64) {
        self.bodies_per_second.set(rate);
    }
    pub fn set_blocks_per_second(&self, rate: f64) {
        self.blocks_per_second.set(rate);
    }

    // Batch timing setters
    pub fn set_batch_body_download_ms(&self, ms: f64) {
        self.batch_body_download_ms.set(ms);
    }
    pub fn set_batch_execution_ms(&self, ms: f64) {
        self.batch_execution_ms.set(ms);
    }
    pub fn set_batch_merkle_ms(&self, ms: f64) {
        self.batch_merkle_ms.set(ms);
    }
    pub fn set_batch_store_ms(&self, ms: f64) {
        self.batch_store_ms.set(ms);
    }
    pub fn set_batch_total_ms(&self, ms: f64) {
        self.batch_total_ms.set(ms);
    }
    pub fn set_batch_size(&self, size: i64) {
        self.batch_size.set(size);
    }

    // Timestamp setters
    pub fn set_header_stage_start_now(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        self.header_stage_start_timestamp.set(now);
    }
    pub fn set_execution_stage_start_now(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        self.execution_stage_start_timestamp.set(now);
    }

    /// Reset gauges at the start of a new sync cycle so stale data doesn't persist
    pub fn reset_cycle(&self) {
        self.blocks_executed.set(0);
        self.blocks_total.set(0);
        self.blocks_per_second.set(0.0);
        self.bodies_per_second.set(0.0);
        self.headers_per_second.set(0.0);
        self.lowest_header.set(0);
        self.target_block.set(0);
        self.batch_body_download_ms.set(0.0);
        self.batch_execution_ms.set(0.0);
        self.batch_merkle_ms.set(0.0);
        self.batch_store_ms.set(0.0);
        self.batch_total_ms.set(0.0);
        self.batch_size.set(0);
        self.header_stage_start_timestamp.set(0.0);
        self.execution_stage_start_timestamp.set(0.0);
    }

    // Reliability setters
    pub fn inc_header_failures(&self) {
        self.header_failures.inc();
    }
    pub fn inc_body_failures(&self) {
        self.body_failures.inc();
    }
    pub fn inc_cycles_started(&self) {
        self.cycles_started.inc();
    }
    pub fn inc_cycles_completed(&self) {
        self.cycles_completed.inc();
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        let metrics: Vec<Box<dyn prometheus::core::Collector>> = vec![
            Box::new(self.stage.clone()),
            Box::new(self.target_block.clone()),
            Box::new(self.lowest_header.clone()),
            Box::new(self.headers_downloaded.clone()),
            Box::new(self.bodies_downloaded.clone()),
            Box::new(self.blocks_executed.clone()),
            Box::new(self.blocks_total.clone()),
            Box::new(self.headers_per_second.clone()),
            Box::new(self.bodies_per_second.clone()),
            Box::new(self.blocks_per_second.clone()),
            Box::new(self.batch_body_download_ms.clone()),
            Box::new(self.batch_execution_ms.clone()),
            Box::new(self.batch_merkle_ms.clone()),
            Box::new(self.batch_store_ms.clone()),
            Box::new(self.batch_total_ms.clone()),
            Box::new(self.batch_size.clone()),
            Box::new(self.header_failures.clone()),
            Box::new(self.body_failures.clone()),
            Box::new(self.header_stage_start_timestamp.clone()),
            Box::new(self.execution_stage_start_timestamp.clone()),
            Box::new(self.cycles_started.clone()),
            Box::new(self.cycles_completed.clone()),
        ];

        for metric in metrics {
            r.register(metric)
                .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        }

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
