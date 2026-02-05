use prometheus::{Encoder, IntCounter, IntGauge, Registry, TextEncoder};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_STORAGE: LazyLock<MetricsStorage> = LazyLock::new(MetricsStorage::default);

#[derive(Debug, Clone)]
pub struct MetricsStorage {
    trie_layer_cache_hits_total: IntCounter,
    trie_layer_cache_misses_total: IntCounter,
    trie_layer_cache_layers: IntGauge,
    trie_db_reads_total: IntCounter,
    trie_db_writes_total: IntCounter,
    trie_db_write_batch_size: IntGauge,
}

impl Default for MetricsStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsStorage {
    pub fn new() -> Self {
        MetricsStorage {
            trie_layer_cache_hits_total: IntCounter::new(
                "trie_layer_cache_hits_total",
                "Total number of trie layer cache hits",
            )
            .expect("Failed to create trie_layer_cache_hits_total metric"),
            trie_layer_cache_misses_total: IntCounter::new(
                "trie_layer_cache_misses_total",
                "Total number of trie layer cache misses",
            )
            .expect("Failed to create trie_layer_cache_misses_total metric"),
            trie_layer_cache_layers: IntGauge::new(
                "trie_layer_cache_layers",
                "Current number of layers in the trie layer cache",
            )
            .expect("Failed to create trie_layer_cache_layers metric"),
            trie_db_reads_total: IntCounter::new(
                "trie_db_reads_total",
                "Total number of trie database reads",
            )
            .expect("Failed to create trie_db_reads_total metric"),
            trie_db_writes_total: IntCounter::new(
                "trie_db_writes_total",
                "Total number of trie database writes",
            )
            .expect("Failed to create trie_db_writes_total metric"),
            trie_db_write_batch_size: IntGauge::new(
                "trie_db_write_batch_size",
                "Size of the last trie database write batch",
            )
            .expect("Failed to create trie_db_write_batch_size metric"),
        }
    }

    pub fn inc_layer_cache_hits(&self) {
        self.trie_layer_cache_hits_total.inc();
    }

    pub fn inc_layer_cache_misses(&self) {
        self.trie_layer_cache_misses_total.inc();
    }

    pub fn set_layer_cache_layers(&self, count: i64) {
        self.trie_layer_cache_layers.set(count);
    }

    pub fn inc_db_reads(&self) {
        self.trie_db_reads_total.inc();
    }

    pub fn inc_db_writes(&self) {
        self.trie_db_writes_total.inc();
    }

    pub fn set_db_write_batch_size(&self, size: i64) {
        self.trie_db_write_batch_size.set(size);
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        r.register(Box::new(self.trie_layer_cache_hits_total.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.trie_layer_cache_misses_total.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.trie_layer_cache_layers.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.trie_db_reads_total.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.trie_db_writes_total.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.trie_db_write_batch_size.clone()))
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
