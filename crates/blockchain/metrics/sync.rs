use prometheus::{IntCounterVec, IntGauge, register_int_counter_vec, register_int_gauge};
use std::sync::LazyLock;

// Metrics defined in this module register into the Prometheus default registry.
// The metrics API exposes them via `gather_default_metrics()`.

pub static METRICS_SYNC: LazyLock<MetricsSync> = LazyLock::new(MetricsSync::default);

#[derive(Debug, Clone)]
pub struct MetricsSync {
    // Gauges — current state
    eligible_peers: IntGauge,
    snap_peers: IntGauge,
    inflight_requests: IntGauge,
    pivot_age_seconds: IntGauge,
    current_phase: IntGauge,

    // Counters — cumulative outcomes
    pivot_updates: IntCounterVec,
    storage_requests: IntCounterVec,
    header_resolution: IntCounterVec,
}

impl Default for MetricsSync {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsSync {
    pub fn new() -> Self {
        MetricsSync {
            eligible_peers: register_int_gauge!(
                "ethrex_sync_eligible_peers",
                "Number of peers eligible for requests (passing can_try_more_requests)"
            )
            .expect("Failed to create eligible_peers metric"),
            snap_peers: register_int_gauge!(
                "ethrex_sync_snap_peers",
                "Number of connected peers supporting the snap protocol"
            )
            .expect("Failed to create snap_peers metric"),
            inflight_requests: register_int_gauge!(
                "ethrex_sync_inflight_requests",
                "Total inflight requests across all peers"
            )
            .expect("Failed to create inflight_requests metric"),
            pivot_age_seconds: register_int_gauge!(
                "ethrex_sync_pivot_age_seconds",
                "Age of the current pivot block in seconds"
            )
            .expect("Failed to create pivot_age_seconds metric"),
            current_phase: register_int_gauge!(
                "ethrex_sync_current_phase",
                "Current snap sync phase (0=idle, 1=headers, 2=account_ranges, 3=account_insertion, 4=storage_ranges, 5=storage_insertion, 6=healing, 7=bytecodes)"
            )
            .expect("Failed to create current_phase metric"),
            pivot_updates: register_int_counter_vec!(
                "ethrex_sync_pivot_updates_total",
                "Total pivot update attempts by outcome",
                &["outcome"]
            )
            .expect("Failed to create pivot_updates metric"),
            storage_requests: register_int_counter_vec!(
                "ethrex_sync_storage_requests_total",
                "Total storage range requests by outcome",
                &["outcome"]
            )
            .expect("Failed to create storage_requests metric"),
            header_resolution: register_int_counter_vec!(
                "ethrex_sync_header_resolution_total",
                "Total header resolution attempts by outcome",
                &["outcome"]
            )
            .expect("Failed to create header_resolution metric"),
        }
    }

    // --- Gauge setters ---

    pub fn set_eligible_peers(&self, count: i64) {
        self.eligible_peers.set(count);
    }

    pub fn set_snap_peers(&self, count: i64) {
        self.snap_peers.set(count);
    }

    pub fn set_inflight_requests(&self, count: i64) {
        self.inflight_requests.set(count);
    }

    pub fn set_pivot_age_seconds(&self, age: i64) {
        self.pivot_age_seconds.set(age);
    }

    pub fn set_current_phase(&self, phase: i64) {
        self.current_phase.set(phase);
    }

    // --- Counter incrementers ---

    pub fn inc_pivot_update(&self, outcome: &str) {
        self.pivot_updates.with_label_values(&[outcome]).inc();
    }

    pub fn inc_storage_request(&self, outcome: &str) {
        self.storage_requests.with_label_values(&[outcome]).inc();
    }

    pub fn inc_header_resolution(&self, outcome: &str) {
        self.header_resolution.with_label_values(&[outcome]).inc();
    }
}
