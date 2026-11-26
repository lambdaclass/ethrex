use prometheus::{
    Histogram, IntCounter, IntCounterVec, IntGauge, register_histogram, register_int_counter,
    register_int_counter_vec, register_int_gauge,
};
use std::sync::LazyLock;

// Metrics defined in this module register into the Prometheus default registry.
// The metrics API exposes them via `gather_default_metrics()`.

/// EIP-8141: claim witnesses in blocks and in the pool.
pub static METRICS_CLAIMS: LazyLock<MetricsClaims> = LazyLock::new(MetricsClaims::default);

// Histogram bucket layout for witness verification times: 50 microseconds base,
// doubling, 16 buckets -> 50 us, 100 us, ..., about 1.6 s. One secp256k1 or
// leanSPHINCS verification takes on the order of 100 us.
const VERIFICATION_BUCKET_START_SECONDS: f64 = 0.00005;
const VERIFICATION_BUCKET_FACTOR: f64 = 2.0;
const VERIFICATION_BUCKET_COUNT: usize = 16;

#[derive(Debug, Clone)]
pub struct MetricsClaims {
    /// Cumulative count of blocks whose claim witnesses were checked.
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
    /// Number of frame transactions with claims in the pool.
    pub pool_transactions: IntGauge,
    /// Total size of the witnesses held in the pool, in bytes.
    pub pool_witness_bytes: IntGauge,
    /// Time spent verifying the witnesses of one transaction on admission.
    pub admission_witness_verification_seconds: Histogram,
    /// Cumulative count of frame transactions with claims admitted to the pool.
    pub admitted_total: IntCounter,
    /// Cumulative count of pooled frame transactions rejected on admission, by
    /// the check that failed.
    pub rejected_total: IntCounterVec,
}

impl Default for MetricsClaims {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsClaims {
    pub fn new() -> Self {
        let verification_buckets = || {
            prometheus::exponential_buckets(
                VERIFICATION_BUCKET_START_SECONDS,
                VERIFICATION_BUCKET_FACTOR,
                VERIFICATION_BUCKET_COUNT,
            )
            .expect("Invalid witness verification histogram bucket params")
        };
        MetricsClaims {
            blocks_total: register_int_counter!(
                "eip8141_claims_blocks_total",
                "Cumulative count of blocks whose claim witnesses (EIP-8141) were checked"
            )
            .expect("Failed to create eip8141_claims_blocks_total metric"),
            block_witnesses: register_int_gauge!(
                "eip8141_claims_block_witnesses",
                "Number of claim witnesses in the most recent block"
            )
            .expect("Failed to create eip8141_claims_block_witnesses metric"),
            block_witness_bytes: register_int_gauge!(
                "eip8141_claims_block_witness_bytes",
                "Total size of the claim witnesses of the most recent block, in bytes"
            )
            .expect("Failed to create eip8141_claims_block_witness_bytes metric"),
            witnesses_total: register_int_counter!(
                "eip8141_claims_witnesses_total",
                "Cumulative count of claim witnesses in checked blocks"
            )
            .expect("Failed to create eip8141_claims_witnesses_total metric"),
            witness_bytes_total: register_int_counter!(
                "eip8141_claims_witness_bytes_total",
                "Cumulative size of the claim witnesses in checked blocks, in bytes"
            )
            .expect("Failed to create eip8141_claims_witness_bytes_total metric"),
            block_witness_verification_seconds: register_histogram!(
                "eip8141_claims_block_witness_verification_seconds",
                "Time spent checking the claim witnesses of one block, in seconds",
                verification_buckets()
            )
            .expect("Failed to create eip8141_claims_block_witness_verification_seconds metric"),
            pool_transactions: register_int_gauge!(
                "eip8141_claims_pool_transactions",
                "Number of frame transactions with claims in the pool"
            )
            .expect("Failed to create eip8141_claims_pool_transactions metric"),
            pool_witness_bytes: register_int_gauge!(
                "eip8141_claims_pool_witness_bytes",
                "Total size of the claim witnesses held in the pool, in bytes"
            )
            .expect("Failed to create eip8141_claims_pool_witness_bytes metric"),
            admission_witness_verification_seconds: register_histogram!(
                "eip8141_claims_admission_witness_verification_seconds",
                "Time spent verifying the witnesses of one transaction on admission to the pool, in seconds",
                verification_buckets()
            )
            .expect("Failed to create eip8141_claims_admission_witness_verification_seconds metric"),
            admitted_total: register_int_counter!(
                "eip8141_claims_admitted_total",
                "Cumulative count of frame transactions with claims admitted to the pool"
            )
            .expect("Failed to create eip8141_claims_admitted_total metric"),
            rejected_total: register_int_counter_vec!(
                "eip8141_claims_rejected_total",
                "Cumulative count of pooled frame transactions rejected on admission, by the check that failed",
                &["reason"]
            )
            .expect("Failed to create eip8141_claims_rejected_total metric"),
        }
    }
}
