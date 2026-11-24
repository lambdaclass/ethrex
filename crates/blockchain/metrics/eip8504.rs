use prometheus::{
    Histogram, IntCounter, IntCounterVec, IntGauge, register_histogram, register_int_counter,
    register_int_counter_vec, register_int_gauge,
};
use std::sync::LazyLock;

// Metrics defined in this module register into the Prometheus default registry.
// The metrics API exposes them via `gather_default_metrics()`.

/// EIP-8504: dependency witnesses in blocks, in the pool and on the network.
pub static METRICS_EIP8504: LazyLock<MetricsEip8504> = LazyLock::new(MetricsEip8504::default);

// Histogram bucket layout for witness verification times: 50 microseconds base,
// doubling, 16 buckets -> 50 us, 100 us, ..., about 1.6 s. One leanSPHINCS
// verification takes on the order of 100 us.
const VERIFICATION_BUCKET_START_SECONDS: f64 = 0.00005;
const VERIFICATION_BUCKET_FACTOR: f64 = 2.0;
const VERIFICATION_BUCKET_COUNT: usize = 16;

#[derive(Debug, Clone)]
pub struct MetricsEip8504 {
    /// Cumulative count of blocks whose dependency witnesses were checked.
    pub blocks_total: IntCounter,
    /// Number of witnesses in the most recent block (per-block snapshot).
    pub block_witnesses: IntGauge,
    /// Total size of the witnesses of the most recent block, in bytes.
    pub block_witness_bytes: IntGauge,
    /// Cumulative count of witnesses in checked blocks.
    pub witnesses_total: IntCounter,
    /// Cumulative size of the witnesses in checked blocks, in bytes.
    pub witness_bytes_total: IntCounter,
    /// Time spent checking the witnesses of one block.
    pub block_witness_verification_seconds: Histogram,
    /// Number of dependency transactions in the pool.
    pub pool_transactions: IntGauge,
    /// Total size of the witnesses held in the pool, in bytes.
    pub pool_witness_bytes: IntGauge,
    /// Time spent verifying the witnesses of one transaction on admission.
    pub admission_witness_verification_seconds: Histogram,
    /// Cumulative count of dependency transactions admitted to the pool.
    pub admitted_total: IntCounter,
    /// Cumulative count of dependency transactions rejected on admission, by the
    /// check that failed.
    pub rejected_total: IntCounterVec,
    /// Cumulative count of wrappers announced to peers.
    pub wrappers_announced_total: IntCounter,
    /// Cumulative count of wrappers sent to peers that requested them.
    pub wrappers_served_total: IntCounter,
    /// Cumulative count of wrappers received from peers.
    pub wrappers_received_total: IntCounter,
}

impl Default for MetricsEip8504 {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsEip8504 {
    pub fn new() -> Self {
        let verification_buckets = || {
            prometheus::exponential_buckets(
                VERIFICATION_BUCKET_START_SECONDS,
                VERIFICATION_BUCKET_FACTOR,
                VERIFICATION_BUCKET_COUNT,
            )
            .expect("Invalid witness verification histogram bucket params")
        };
        MetricsEip8504 {
            blocks_total: register_int_counter!(
                "eip8504_blocks_total",
                "Cumulative count of blocks whose dependency witnesses (EIP-8504) were checked"
            )
            .expect("Failed to create eip8504_blocks_total metric"),
            block_witnesses: register_int_gauge!(
                "eip8504_block_witnesses",
                "Number of dependency witnesses in the most recent block"
            )
            .expect("Failed to create eip8504_block_witnesses metric"),
            block_witness_bytes: register_int_gauge!(
                "eip8504_block_witness_bytes",
                "Total size of the dependency witnesses of the most recent block, in bytes"
            )
            .expect("Failed to create eip8504_block_witness_bytes metric"),
            witnesses_total: register_int_counter!(
                "eip8504_witnesses_total",
                "Cumulative count of dependency witnesses in checked blocks"
            )
            .expect("Failed to create eip8504_witnesses_total metric"),
            witness_bytes_total: register_int_counter!(
                "eip8504_witness_bytes_total",
                "Cumulative size of the dependency witnesses in checked blocks, in bytes"
            )
            .expect("Failed to create eip8504_witness_bytes_total metric"),
            block_witness_verification_seconds: register_histogram!(
                "eip8504_block_witness_verification_seconds",
                "Time spent checking the dependency witnesses of one block, in seconds",
                verification_buckets()
            )
            .expect("Failed to create eip8504_block_witness_verification_seconds metric"),
            pool_transactions: register_int_gauge!(
                "eip8504_pool_transactions",
                "Number of dependency transactions in the pool"
            )
            .expect("Failed to create eip8504_pool_transactions metric"),
            pool_witness_bytes: register_int_gauge!(
                "eip8504_pool_witness_bytes",
                "Total size of the dependency witnesses held in the pool, in bytes"
            )
            .expect("Failed to create eip8504_pool_witness_bytes metric"),
            admission_witness_verification_seconds: register_histogram!(
                "eip8504_admission_witness_verification_seconds",
                "Time spent verifying the witnesses of one transaction on admission to the pool, in seconds",
                verification_buckets()
            )
            .expect("Failed to create eip8504_admission_witness_verification_seconds metric"),
            admitted_total: register_int_counter!(
                "eip8504_admitted_total",
                "Cumulative count of dependency transactions admitted to the pool"
            )
            .expect("Failed to create eip8504_admitted_total metric"),
            rejected_total: register_int_counter_vec!(
                "eip8504_rejected_total",
                "Cumulative count of dependency transactions rejected on admission, by the check that failed",
                &["reason"]
            )
            .expect("Failed to create eip8504_rejected_total metric"),
            wrappers_announced_total: register_int_counter!(
                "eip8504_wrappers_announced_total",
                "Cumulative count of wrappers announced to peers"
            )
            .expect("Failed to create eip8504_wrappers_announced_total metric"),
            wrappers_served_total: register_int_counter!(
                "eip8504_wrappers_served_total",
                "Cumulative count of wrappers sent to peers that requested them"
            )
            .expect("Failed to create eip8504_wrappers_served_total metric"),
            wrappers_received_total: register_int_counter!(
                "eip8504_wrappers_received_total",
                "Cumulative count of wrappers received from peers"
            )
            .expect("Failed to create eip8504_wrappers_received_total metric"),
        }
    }
}
