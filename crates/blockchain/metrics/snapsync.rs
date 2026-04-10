use prometheus::{Encoder, Gauge, IntCounter, IntGauge, Registry, TextEncoder};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_SNAPSYNC: LazyLock<MetricsSnapSync> = LazyLock::new(MetricsSnapSync::default);

#[derive(Debug, Clone)]
pub struct MetricsSnapSync {
    // Phase
    stage: IntGauge,
    target_block: IntGauge,

    // Headers
    headers_downloaded: IntGauge,
    headers_total: IntGauge,
    headers_per_second: Gauge,
    headers_stage_start_timestamp: Gauge,

    // Accounts
    accounts_downloaded: IntGauge,
    accounts_inserted: IntGauge,
    accounts_per_second: Gauge,
    accounts_stage_start_timestamp: Gauge,

    // Storage
    storage_downloaded: IntCounter,
    storage_inserted: IntCounter,
    storage_per_second: Gauge,
    storage_stage_start_timestamp: Gauge,

    // Healing
    state_leaves_healed: IntGauge,
    storage_leaves_healed: IntGauge,
    healing_per_second: Gauge,
    healing_stage_start_timestamp: Gauge,

    // Bytecodes
    bytecodes_downloaded: IntGauge,
    bytecodes_total: IntGauge,
    bytecodes_per_second: Gauge,
    bytecodes_stage_start_timestamp: Gauge,
}

impl Default for MetricsSnapSync {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsSnapSync {
    pub fn new() -> Self {
        MetricsSnapSync {
            stage: IntGauge::new(
                "snapsync_stage",
                "Current snap sync stage: 0=none, 1=healing_storage, 2=healing_state, 3=bytecodes, 4=account_ranges, 5=storage_ranges, 6=headers, 7=inserting_storage, 8=inserting_accounts, 9=inserting_accounts_nodb",
            ).expect("Failed to create snapsync_stage"),
            target_block: IntGauge::new(
                "snapsync_target_block",
                "Snap sync pivot block number",
            ).expect("Failed to create snapsync_target_block"),

            // Headers
            headers_downloaded: IntGauge::new(
                "snapsync_headers_downloaded",
                "Headers downloaded so far",
            ).expect("Failed to create snapsync_headers_downloaded"),
            headers_total: IntGauge::new(
                "snapsync_headers_total",
                "Total headers to download",
            ).expect("Failed to create snapsync_headers_total"),
            headers_per_second: Gauge::new(
                "snapsync_headers_per_second",
                "Header download rate (5m avg via internal calc)",
            ).expect("Failed to create snapsync_headers_per_second"),
            headers_stage_start_timestamp: Gauge::new(
                "snapsync_headers_stage_start_timestamp",
                "Unix timestamp when header download began",
            ).expect("Failed to create snapsync_headers_stage_start_timestamp"),

            // Accounts
            accounts_downloaded: IntGauge::new(
                "snapsync_accounts_downloaded",
                "Account ranges downloaded so far",
            ).expect("Failed to create snapsync_accounts_downloaded"),
            accounts_inserted: IntGauge::new(
                "snapsync_accounts_inserted",
                "Account ranges inserted to DB so far",
            ).expect("Failed to create snapsync_accounts_inserted"),
            accounts_per_second: Gauge::new(
                "snapsync_accounts_per_second",
                "Account download/insert rate",
            ).expect("Failed to create snapsync_accounts_per_second"),
            accounts_stage_start_timestamp: Gauge::new(
                "snapsync_accounts_stage_start_timestamp",
                "Unix timestamp when account download began",
            ).expect("Failed to create snapsync_accounts_stage_start_timestamp"),

            // Storage
            storage_downloaded: IntCounter::new(
                "snapsync_storage_downloaded",
                "Storage leaves downloaded",
            ).expect("Failed to create snapsync_storage_downloaded"),
            storage_inserted: IntCounter::new(
                "snapsync_storage_inserted",
                "Storage leaves inserted to DB",
            ).expect("Failed to create snapsync_storage_inserted"),
            storage_per_second: Gauge::new(
                "snapsync_storage_per_second",
                "Storage download/insert rate",
            ).expect("Failed to create snapsync_storage_per_second"),
            storage_stage_start_timestamp: Gauge::new(
                "snapsync_storage_stage_start_timestamp",
                "Unix timestamp when storage download began",
            ).expect("Failed to create snapsync_storage_stage_start_timestamp"),

            // Healing
            state_leaves_healed: IntGauge::new(
                "snapsync_state_leaves_healed",
                "State trie leaves healed",
            ).expect("Failed to create snapsync_state_leaves_healed"),
            storage_leaves_healed: IntGauge::new(
                "snapsync_storage_leaves_healed",
                "Storage trie leaves healed",
            ).expect("Failed to create snapsync_storage_leaves_healed"),
            healing_per_second: Gauge::new(
                "snapsync_healing_per_second",
                "Healing rate (leaves/sec)",
            ).expect("Failed to create snapsync_healing_per_second"),
            healing_stage_start_timestamp: Gauge::new(
                "snapsync_healing_stage_start_timestamp",
                "Unix timestamp when healing began",
            ).expect("Failed to create snapsync_healing_stage_start_timestamp"),

            // Bytecodes
            bytecodes_downloaded: IntGauge::new(
                "snapsync_bytecodes_downloaded",
                "Bytecodes downloaded so far",
            ).expect("Failed to create snapsync_bytecodes_downloaded"),
            bytecodes_total: IntGauge::new(
                "snapsync_bytecodes_total",
                "Total bytecodes to download",
            ).expect("Failed to create snapsync_bytecodes_total"),
            bytecodes_per_second: Gauge::new(
                "snapsync_bytecodes_per_second",
                "Bytecode download rate",
            ).expect("Failed to create snapsync_bytecodes_per_second"),
            bytecodes_stage_start_timestamp: Gauge::new(
                "snapsync_bytecodes_stage_start_timestamp",
                "Unix timestamp when bytecode download began",
            ).expect("Failed to create snapsync_bytecodes_stage_start_timestamp"),
        }
    }

    // Phase setters
    pub fn set_stage(&self, stage: i64) { self.stage.set(stage); }
    pub fn set_target_block(&self, block: u64) { self.target_block.set(block.cast_signed()); }

    // Headers
    pub fn set_headers_downloaded(&self, n: u64) { self.headers_downloaded.set(n.cast_signed()); }
    pub fn set_headers_total(&self, n: u64) { self.headers_total.set(n.cast_signed()); }
    pub fn set_headers_per_second(&self, r: f64) { self.headers_per_second.set(r); }
    pub fn set_headers_stage_start_now(&self) { self.headers_stage_start_timestamp.set(now_secs()); }

    // Accounts
    pub fn set_accounts_downloaded(&self, n: u64) { self.accounts_downloaded.set(n.cast_signed()); }
    pub fn set_accounts_inserted(&self, n: u64) { self.accounts_inserted.set(n.cast_signed()); }
    pub fn set_accounts_per_second(&self, r: f64) { self.accounts_per_second.set(r); }
    pub fn set_accounts_stage_start_now(&self) { self.accounts_stage_start_timestamp.set(now_secs()); }

    // Storage
    pub fn inc_storage_downloaded(&self, n: u64) { self.storage_downloaded.inc_by(n); }
    pub fn inc_storage_inserted(&self, n: u64) { self.storage_inserted.inc_by(n); }
    pub fn set_storage_per_second(&self, r: f64) { self.storage_per_second.set(r); }
    pub fn set_storage_stage_start_now(&self) { self.storage_stage_start_timestamp.set(now_secs()); }

    // Healing
    pub fn set_state_leaves_healed(&self, n: u64) { self.state_leaves_healed.set(n.cast_signed()); }
    pub fn set_storage_leaves_healed(&self, n: u64) { self.storage_leaves_healed.set(n.cast_signed()); }
    pub fn set_healing_per_second(&self, r: f64) { self.healing_per_second.set(r); }
    pub fn set_healing_stage_start_now(&self) { self.healing_stage_start_timestamp.set(now_secs()); }

    // Bytecodes
    pub fn set_bytecodes_downloaded(&self, n: u64) { self.bytecodes_downloaded.set(n.cast_signed()); }
    pub fn set_bytecodes_total(&self, n: u64) { self.bytecodes_total.set(n.cast_signed()); }
    pub fn set_bytecodes_per_second(&self, r: f64) { self.bytecodes_per_second.set(r); }
    pub fn set_bytecodes_stage_start_now(&self) { self.bytecodes_stage_start_timestamp.set(now_secs()); }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        let metrics: Vec<Box<dyn prometheus::core::Collector>> = vec![
            Box::new(self.stage.clone()),
            Box::new(self.target_block.clone()),
            Box::new(self.headers_downloaded.clone()),
            Box::new(self.headers_total.clone()),
            Box::new(self.headers_per_second.clone()),
            Box::new(self.headers_stage_start_timestamp.clone()),
            Box::new(self.accounts_downloaded.clone()),
            Box::new(self.accounts_inserted.clone()),
            Box::new(self.accounts_per_second.clone()),
            Box::new(self.accounts_stage_start_timestamp.clone()),
            Box::new(self.storage_downloaded.clone()),
            Box::new(self.storage_inserted.clone()),
            Box::new(self.storage_per_second.clone()),
            Box::new(self.storage_stage_start_timestamp.clone()),
            Box::new(self.state_leaves_healed.clone()),
            Box::new(self.storage_leaves_healed.clone()),
            Box::new(self.healing_per_second.clone()),
            Box::new(self.healing_stage_start_timestamp.clone()),
            Box::new(self.bytecodes_downloaded.clone()),
            Box::new(self.bytecodes_total.clone()),
            Box::new(self.bytecodes_per_second.clone()),
            Box::new(self.bytecodes_stage_start_timestamp.clone()),
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
        Ok(String::from_utf8(buffer)?)
    }
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
