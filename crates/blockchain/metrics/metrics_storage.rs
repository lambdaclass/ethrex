use prometheus::{Encoder, IntGauge, Registry, TextEncoder};
use rocksdb::DB;
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_STORAGE: LazyLock<MetricsStorage> = LazyLock::new(MetricsStorage::default);

#[derive(Debug, Clone)]
pub struct MetricsStorage {
    pub total_sst_bytes: IntGauge,   // rocksdb.total-sst-files-size (DB-scoped)
    pub total_log_bytes: IntGauge,   // rocksdb.total-log-size (DB-scoped)
    pub total_on_disk_bytes: IntGauge, // SST + WAL (exported for convenience)
}

impl Default for MetricsStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsStorage {
    pub fn new() -> Self {
        Self {
            total_sst_bytes: IntGauge::new(
                "rocksdb_total_sst_files_size_bytes",
                "Total size of active SST files (DB-wide)",
            ).unwrap(),
            total_log_bytes: IntGauge::new(
                "rocksdb_total_log_size_bytes",
                "Total size of WAL files (DB-wide)",
            ).unwrap(),
            total_on_disk_bytes: IntGauge::new(
                "rocksdb_total_on_disk_bytes",
                "Total on-disk size of RocksDB (SST + WAL, DB-wide)",
            ).unwrap(),
        }
    }

    #[inline]
    fn set_db_prop(&self, db: &DB, prop: &str, g: &IntGauge) -> Option<u64> {
        match db.property_int_value(prop) {
            Ok(Some(v)) => { g.set(v as i64); Some(v) }
            _ => None,
        }
    }

    /// Call this where you already tick metrics (no CF names needed).
    pub fn scrape(&self, db: &DB) {
        let sst = self.set_db_prop(db, "rocksdb.total-sst-files-size", &self.total_sst_bytes).unwrap_or(0);
        let wal = self.set_db_prop(db, "rocksdb.total-log-size", &self.total_log_bytes).unwrap_or(0);
        // Export a convenient sum too (PromQL sum works, but this is handy for dashboards/alerts).
        let total = sst.saturating_add(wal);
        self.total_on_disk_bytes.set(total as i64);
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();
        r.register(Box::new(self.total_sst_bytes.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.total_log_bytes.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.total_on_disk_bytes.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        let metric_families = r.gather();
        let encoder = TextEncoder::new();
        let mut buf = Vec::new();
        encoder.encode(&metric_families, &mut buf)
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        Ok(String::from_utf8(buf)?)
    }
}