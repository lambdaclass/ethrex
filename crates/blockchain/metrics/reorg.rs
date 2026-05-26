use prometheus::{Encoder, Histogram, HistogramOpts, IntCounter, IntGauge, Registry, TextEncoder};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_REORG: LazyLock<MetricsReorg> = LazyLock::new(MetricsReorg::new);

#[derive(Debug, Clone)]
pub struct MetricsReorg {
    /// Current number of entries in the installed overlay (0 when no reorg in progress).
    pub overlay_entries: IntGauge,
    /// Current byte size of overlay key+value data.
    pub overlay_bytes: IntGauge,
    /// `highest - lowest + 1` of the STATE_HISTORY column family, or 0 if empty.
    pub journal_length: IntGauge,
    /// Distribution of attempted reorg depths.
    pub reorg_depth_hist: Histogram,
    /// Duration of the first-commit reconciliation in seconds.
    pub reconcile_duration_hist: Histogram,
    /// Total deep reorgs initiated.
    pub deep_reorg_attempts_total: IntCounter,
    /// Total deep reorgs that completed successfully.
    pub deep_reorg_success_total: IntCounter,
    /// Total deep reorgs that aborted via `AbortReorgGuard`.
    pub deep_reorg_aborts_total: IntCounter,
}

impl MetricsReorg {
    pub fn new() -> Self {
        MetricsReorg {
            overlay_entries: IntGauge::new(
                "ethrex_reorg_overlay_entries",
                "Current number of entries in the installed overlay (0 when no reorg in progress)",
            )
            .expect("Failed to create ethrex_reorg_overlay_entries metric"),
            overlay_bytes: IntGauge::new(
                "ethrex_reorg_overlay_bytes",
                "Current byte size of overlay key+value data",
            )
            .expect("Failed to create ethrex_reorg_overlay_bytes metric"),
            journal_length: IntGauge::new(
                "ethrex_reorg_journal_length",
                "Span of STATE_HISTORY column family (highest - lowest + 1), or 0 if empty",
            )
            .expect("Failed to create ethrex_reorg_journal_length metric"),
            reorg_depth_hist: Histogram::with_opts(
                HistogramOpts::new(
                    "ethrex_reorg_depth",
                    "Distribution of attempted reorg depths",
                )
                .buckets(vec![
                    1.0, 10.0, 32.0, 64.0, 128.0, 256.0, 512.0, 1024.0, 4096.0, 16384.0,
                ]),
            )
            .expect("Failed to create ethrex_reorg_depth metric"),
            reconcile_duration_hist: Histogram::with_opts(
                HistogramOpts::new(
                    "ethrex_reorg_reconcile_duration_seconds",
                    "Duration of the first-commit reconciliation in seconds",
                )
                .buckets(vec![0.001, 0.01, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0]),
            )
            .expect("Failed to create ethrex_reorg_reconcile_duration_seconds metric"),
            deep_reorg_attempts_total: IntCounter::new(
                "ethrex_deep_reorg_attempts_total",
                "Total deep reorgs initiated",
            )
            .expect("Failed to create ethrex_deep_reorg_attempts_total metric"),
            deep_reorg_success_total: IntCounter::new(
                "ethrex_deep_reorg_success_total",
                "Total deep reorgs that completed successfully",
            )
            .expect("Failed to create ethrex_deep_reorg_success_total metric"),
            deep_reorg_aborts_total: IntCounter::new(
                "ethrex_deep_reorg_aborts_total",
                "Total deep reorgs that aborted via AbortReorgGuard",
            )
            .expect("Failed to create ethrex_deep_reorg_aborts_total metric"),
        }
    }
}

impl Default for MetricsReorg {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsReorg {
    /// Register all reorg metrics into a fresh registry and encode them as a
    /// Prometheus text payload. Mirrors `MetricsBlocks::gather_metrics`.
    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();
        r.register(Box::new(self.overlay_entries.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.overlay_bytes.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.journal_length.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.reorg_depth_hist.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.reconcile_duration_hist.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.deep_reorg_attempts_total.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.deep_reorg_success_total.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.deep_reorg_aborts_total.clone()))
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
