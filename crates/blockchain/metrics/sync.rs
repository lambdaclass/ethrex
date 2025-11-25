use prometheus::{Encoder, IntGauge, Registry, TextEncoder};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_SYNC: LazyLock<MetricsSync> = LazyLock::new(MetricsSync::default);

#[derive(Debug, Clone)]
pub struct MetricsSync {
    pub peer_count: IntGauge,
    pub headers_downloaded: IntGauge,
    pub headers_total: IntGauge,
    pub account_leaves_downloaded: IntGauge,
    pub account_leaves_inserted: IntGauge,
    pub sync_step: IntGauge,
}

impl Default for MetricsSync {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsSync {
    pub fn new() -> Self {
        MetricsSync {
            peer_count: IntGauge::new(
                "ethrex_p2p_peer_count",
                "Number of connected peers",
            )
            .unwrap(),
            headers_downloaded: IntGauge::new(
                "ethrex_sync_headers_downloaded",
                "Number of block headers downloaded",
            )
            .unwrap(),
            headers_total: IntGauge::new(
                "ethrex_sync_headers_total",
                "Total number of block headers to download (target)",
            )
            .unwrap(),
            account_leaves_downloaded: IntGauge::new(
                "ethrex_sync_account_leaves_downloaded",
                "Number of account trie leaves downloaded",
            )
            .unwrap(),
            account_leaves_inserted: IntGauge::new(
                "ethrex_sync_account_leaves_inserted",
                "Number of account trie leaves inserted into the DB",
            )
            .unwrap(),
            sync_step: IntGauge::new(
                "ethrex_sync_step",
                "Current sync step (0=None, 1=HealingStorage, 2=HealingState, 3=RequestingBytecodes, 4=RequestingAccountRanges, 5=RequestingStorageRanges, 6=DownloadingHeaders, 7=InsertingStorageRanges, 8=InsertingAccountRanges, 9=InsertingAccountRangesNoDb)",
            )
            .unwrap(),
        }
    }

    pub fn set_peer_count(&self, count: u64) {
        self.peer_count.set(count as i64);
    }

    pub fn set_sync_step(&self, step: u64) {
        self.sync_step.set(step as i64);
    }

    pub fn set_headers_progress(&self, downloaded: u64, total: u64) {
        self.headers_downloaded.set(downloaded as i64);
        self.headers_total.set(total as i64);
    }

    pub fn set_account_leaves_progress(&self, downloaded: u64, inserted: u64) {
        self.account_leaves_downloaded.set(downloaded as i64);
        self.account_leaves_inserted.set(inserted as i64);
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();
        r.register(Box::new(self.peer_count.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.headers_downloaded.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.headers_total.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.account_leaves_downloaded.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.account_leaves_inserted.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.sync_step.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        let encoder = TextEncoder::new();
        let metric_families = r.gather();
        let mut buffer = Vec::new();
        encoder
            .encode(&metric_families, &mut buffer)
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        Ok(String::from_utf8(buffer)?)
    }
}
