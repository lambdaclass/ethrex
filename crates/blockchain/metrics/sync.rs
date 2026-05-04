use prometheus::{
    IntCounterVec, IntGauge, IntGaugeVec, register_int_counter_vec, register_int_gauge,
    register_int_gauge_vec,
};
use std::sync::LazyLock;

// Metrics defined in this module register into the Prometheus default registry.
// The metrics API exposes them via `gather_default_metrics()`.

pub static METRICS_SYNC: LazyLock<MetricsSync> = LazyLock::new(MetricsSync::default);

#[derive(Debug, Clone)]
pub struct MetricsSync {
    // --- Current state (gauges) ---
    pub stage: IntGauge,
    pub pivot_block: IntGauge,
    pub eligible_peers: IntGauge,
    pub snap_peers: IntGauge,
    pub inflight_requests: IntGauge,
    pub pivot_age_seconds: IntGauge,
    pub pivot_timestamp: IntGauge,
    pub phase_start_timestamp: IntGaugeVec,

    // --- Progress counters (gauges set from METRICS atomics) ---
    // Use rate() in Grafana to derive throughput.
    pub headers_downloaded: IntGauge,
    pub headers_total: IntGauge,
    pub accounts_downloaded: IntGauge,
    pub accounts_inserted: IntGauge,
    pub storage_downloaded: IntGauge,
    pub storage_inserted: IntGauge,
    pub state_leaves_healed: IntGauge,
    pub storage_leaves_healed: IntGauge,
    pub bytecodes_downloaded: IntGauge,
    pub bytecodes_total: IntGauge,

    // --- Outcome counters (counter vecs) ---
    pub pivot_updates: IntCounterVec,
    pub storage_requests: IntCounterVec,
    pub header_resolution: IntCounterVec,
}

impl Default for MetricsSync {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsSync {
    pub fn new() -> Self {
        MetricsSync {
            // Current state
            stage: register_int_gauge!(
                "ethrex_sync_stage",
                "Current snap sync stage (0=idle, 1=headers, 2=account_ranges, 3=account_insertion, 4=storage_ranges, 5=storage_insertion, 6=state_healing, 7=storage_healing, 8=bytecodes)"
            )
            .expect("Failed to create ethrex_sync_stage"),
            pivot_block: register_int_gauge!(
                "ethrex_sync_pivot_block",
                "Current pivot block number"
            )
            .expect("Failed to create ethrex_sync_pivot_block"),
            eligible_peers: register_int_gauge!(
                "ethrex_sync_eligible_peers",
                "Number of peers eligible for requests"
            )
            .expect("Failed to create ethrex_sync_eligible_peers"),
            snap_peers: register_int_gauge!(
                "ethrex_sync_snap_peers",
                "Number of connected peers supporting the snap protocol"
            )
            .expect("Failed to create ethrex_sync_snap_peers"),
            inflight_requests: register_int_gauge!(
                "ethrex_sync_inflight_requests",
                "Total inflight requests across all peers"
            )
            .expect("Failed to create ethrex_sync_inflight_requests"),
            pivot_age_seconds: register_int_gauge!(
                "ethrex_sync_pivot_age_seconds",
                "Age of the current pivot block in seconds"
            )
            .expect("Failed to create ethrex_sync_pivot_age_seconds"),
            pivot_timestamp: register_int_gauge!(
                "ethrex_sync_pivot_timestamp",
                "Unix timestamp of the current pivot block (use time() - this for age in Grafana)"
            )
            .expect("Failed to create ethrex_sync_pivot_timestamp"),

            phase_start_timestamp: register_int_gauge_vec!(
                "ethrex_sync_phase_start_timestamp",
                "Unix timestamp when each phase began (use time() - this for elapsed in Grafana)",
                &["phase"]
            )
            .expect("Failed to create ethrex_sync_phase_start_timestamp"),

            // Progress (set periodically from METRICS atomics)
            headers_downloaded: register_int_gauge!(
                "ethrex_sync_headers_downloaded",
                "Headers downloaded so far"
            )
            .expect("Failed to create ethrex_sync_headers_downloaded"),
            headers_total: register_int_gauge!(
                "ethrex_sync_headers_total",
                "Total headers to download (pivot block number)"
            )
            .expect("Failed to create ethrex_sync_headers_total"),
            accounts_downloaded: register_int_gauge!(
                "ethrex_sync_accounts_downloaded",
                "Account ranges downloaded from peers"
            )
            .expect("Failed to create ethrex_sync_accounts_downloaded"),
            accounts_inserted: register_int_gauge!(
                "ethrex_sync_accounts_inserted",
                "Accounts inserted into storage"
            )
            .expect("Failed to create ethrex_sync_accounts_inserted"),
            storage_downloaded: register_int_gauge!(
                "ethrex_sync_storage_downloaded",
                "Storage leaves downloaded from peers"
            )
            .expect("Failed to create ethrex_sync_storage_downloaded"),
            storage_inserted: register_int_gauge!(
                "ethrex_sync_storage_inserted",
                "Storage leaves inserted into storage"
            )
            .expect("Failed to create ethrex_sync_storage_inserted"),
            state_leaves_healed: register_int_gauge!(
                "ethrex_sync_state_leaves_healed",
                "State trie leaves healed"
            )
            .expect("Failed to create ethrex_sync_state_leaves_healed"),
            storage_leaves_healed: register_int_gauge!(
                "ethrex_sync_storage_leaves_healed",
                "Storage trie leaves healed"
            )
            .expect("Failed to create ethrex_sync_storage_leaves_healed"),
            bytecodes_downloaded: register_int_gauge!(
                "ethrex_sync_bytecodes_downloaded",
                "Bytecodes downloaded so far"
            )
            .expect("Failed to create ethrex_sync_bytecodes_downloaded"),
            bytecodes_total: register_int_gauge!(
                "ethrex_sync_bytecodes_total",
                "Total bytecodes to download"
            )
            .expect("Failed to create ethrex_sync_bytecodes_total"),

            // Outcome counters
            pivot_updates: register_int_counter_vec!(
                "ethrex_sync_pivot_updates_total",
                "Total pivot update attempts by outcome",
                &["outcome"]
            )
            .expect("Failed to create ethrex_sync_pivot_updates_total"),
            storage_requests: register_int_counter_vec!(
                "ethrex_sync_storage_requests_total",
                "Total storage range requests by outcome",
                &["outcome"]
            )
            .expect("Failed to create ethrex_sync_storage_requests_total"),
            header_resolution: register_int_counter_vec!(
                "ethrex_sync_header_resolution_total",
                "Total header resolution attempts by outcome",
                &["outcome"]
            )
            .expect("Failed to create ethrex_sync_header_resolution_total"),
        }
    }

    // --- Gauge setters (used by p2p sync code directly) ---

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
        self.stage.set(phase);
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
