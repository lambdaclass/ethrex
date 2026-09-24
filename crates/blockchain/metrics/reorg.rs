use prometheus::{Encoder, Histogram, HistogramOpts, IntCounter, IntGauge, Registry, TextEncoder};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_REORG: LazyLock<MetricsReorg> = LazyLock::new(MetricsReorg::new);

/// Prometheus metrics for the deep-reorg path.
///
/// All metrics are exported under the `ethrex_reorg_*` / `ethrex_deep_reorg_*` namespace
/// and available at the `/metrics` endpoint. Operators can use these to detect and
/// diagnose unusual reorg activity:
/// - Gauge metrics (`overlay_entries`, `overlay_bytes`, `journal_length`) reflect current
///   in-flight state and return to zero after a successful reorg completes.
/// - Counter metrics (`deep_reorg_attempts_total`, `deep_reorg_success_total`,
///   `deep_reorg_aborts_total`) are monotonically increasing.
/// - Histogram metrics (`reorg_depth_hist`, `reconcile_duration_hist`) record
///   distribution of individual reorg events.
#[derive(Debug, Clone)]
pub struct MetricsReorg {
    /// Number of entries in the installed overlay, or 0 when no deep reorg is in progress.
    /// A non-zero value means the node is mid-reorg; watch for it staying elevated.
    pub overlay_entries: IntGauge,
    /// Byte size of overlay key+value data, or 0 when no deep reorg is in progress.
    /// Large values indicate a deep reorg spanning many blocks with high state churn.
    pub overlay_bytes: IntGauge,
    /// Span of the `STATE_HISTORY` column family (`highest - lowest + 1`), or 0 if empty.
    /// Reflects how many blocks of reorg history are available; shrinks as finality pruning runs.
    pub journal_length: IntGauge,
    /// Distribution of reorg depths observed at each fork-choice update.
    /// Use to gauge whether peers are producing unexpectedly long reorgs.
    pub reorg_depth_hist: Histogram,
    /// Distribution of first-commit reconciliation latencies in seconds.
    /// Captures the overlay-fold + disk-write cost on the first block of a new chain.
    pub reconcile_duration_hist: Histogram,
    /// Total number of deep-reorg apply attempts (counter). Incremented before overlay install.
    pub deep_reorg_attempts_total: IntCounter,
    /// Total number of deep reorgs that completed successfully (counter).
    pub deep_reorg_success_total: IntCounter,
    /// Total number of deep reorgs that aborted via `AbortReorgGuard` (counter).
    /// A sustained difference from `deep_reorg_attempts_total` indicates persistent failures.
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
