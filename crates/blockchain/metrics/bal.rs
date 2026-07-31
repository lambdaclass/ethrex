use prometheus::{
    Gauge, Histogram, IntCounter, IntGauge, register_gauge, register_histogram,
    register_int_counter, register_int_gauge,
};
use std::sync::LazyLock;

// Metrics defined in this module register into the Prometheus default registry.
// The metrics API exposes them via `gather_default_metrics()`.

pub static METRICS_BAL: LazyLock<MetricsBal> = LazyLock::new(MetricsBal::default);

// Histogram bucket layout for RLP-encoded BAL sizes: 1 KiB base, doubling, 16
// buckets -> 1 KiB, 2 KiB, 4 KiB, ..., 32 MiB. Observed sizes are always >= 1
// byte (the smallest BAL RLP-encodes to a non-empty list), so there is no zero
// floor bucket.
const BAL_SIZE_BUCKET_START_BYTES: f64 = 1024.0;
const BAL_SIZE_BUCKET_FACTOR: f64 = 2.0;
const BAL_SIZE_BUCKET_COUNT: usize = 16;

#[derive(Debug, Clone)]
pub struct MetricsBal {
    /// Cumulative count of BAL-carrying blocks processed (post-Amsterdam).
    pub blocks_total: IntCounter,
    /// RLP-encoded size of the most recent BAL, in bytes (per-block snapshot).
    pub size_bytes: Gauge,
    /// Distribution of RLP-encoded BAL sizes in bytes.
    pub size_bytes_histogram: Histogram,
    /// Number of accounts in the most recent BAL (per-block snapshot).
    pub account_count: IntGauge,
    /// Unique storage slots (writes + reads) in the most recent BAL (per-block snapshot).
    pub slot_count: IntGauge,
}

impl Default for MetricsBal {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsBal {
    pub fn new() -> Self {
        MetricsBal {
            blocks_total: register_int_counter!(
                "bal_blocks_total",
                "Cumulative count of Block Access List (EIP-7928) carrying blocks processed"
            )
            .expect("Failed to create bal_blocks_total metric"),
            size_bytes: register_gauge!(
                "bal_size_bytes",
                "RLP-encoded size of the most recent Block Access List, in bytes"
            )
            .expect("Failed to create bal_size_bytes metric"),
            size_bytes_histogram: register_histogram!(
                "bal_size_bytes_histogram",
                "Distribution of RLP-encoded Block Access List sizes in bytes",
                prometheus::exponential_buckets(
                    BAL_SIZE_BUCKET_START_BYTES,
                    BAL_SIZE_BUCKET_FACTOR,
                    BAL_SIZE_BUCKET_COUNT,
                )
                .expect("Invalid BAL histogram bucket params")
            )
            .expect("Failed to create bal_size_bytes_histogram metric"),
            account_count: register_int_gauge!(
                "bal_account_count",
                "Number of accounts in the most recent Block Access List"
            )
            .expect("Failed to create bal_account_count metric"),
            slot_count: register_int_gauge!(
                "bal_slot_count",
                "Unique storage slots (writes + reads) in the most recent BAL"
            )
            .expect("Failed to create bal_slot_count metric"),
        }
    }
}
