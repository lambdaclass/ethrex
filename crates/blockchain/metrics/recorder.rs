//! Prometheus metrics recorder setup using the `metrics` ecosystem.
//!
//! This module configures the metrics-exporter-prometheus recorder with
//! quantiles for summary metrics (p50, p90, p95, p99, p999).

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::num::NonZeroU32;
use std::sync::OnceLock;
use std::time::Duration;

use crate::MetricsError;

/// Global handle to the Prometheus recorder for rendering metrics
static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Default quantiles exposed for summary metrics
/// These are: min, p50, p90, p95, p99, p999, max
pub const DEFAULT_QUANTILES: &[f64] = &[0.0, 0.5, 0.9, 0.95, 0.99, 0.999, 1.0];

/// Default bucket duration for the sliding time window
/// Using shorter buckets for more responsive metrics
pub const DEFAULT_BUCKET_DURATION: Duration = Duration::from_secs(30); // 30 seconds per bucket

/// Default number of buckets in the sliding window
/// Total window = bucket_count * bucket_duration = 4 * 30s = 2 minutes
/// This provides more responsive quantiles that track recent block variations
pub const DEFAULT_BUCKET_COUNT: NonZeroU32 = unsafe { NonZeroU32::new_unchecked(4) };

/// Initialize the global metrics recorder with Prometheus exporter.
///
/// This sets up the `metrics` crate's global recorder to export metrics
/// in Prometheus format with summary quantiles (p50, p90, p95, p99, p999).
///
/// Should be called once at application startup before any metrics are recorded.
pub fn initialize_metrics_recorder() -> Result<(), MetricsError> {
    let builder = PrometheusBuilder::new()
        .set_quantiles(DEFAULT_QUANTILES)
        .map_err(|e| MetricsError::PrometheusErr(format!("Failed to set quantiles: {e}")))?;

    let builder = builder
        .set_bucket_duration(DEFAULT_BUCKET_DURATION)
        .map_err(|e| MetricsError::PrometheusErr(format!("Failed to set bucket duration: {e}")))?
        .set_bucket_count(DEFAULT_BUCKET_COUNT);

    // Build just the recorder (not the full exporter with HTTP server)
    let recorder = builder.build_recorder();

    // Get the handle for rendering metrics
    let handle = recorder.handle();

    // Install the recorder globally
    metrics::set_global_recorder(recorder)
        .map_err(|e| MetricsError::PrometheusErr(format!("Failed to set global recorder: {e}")))?;

    // Store the handle for later rendering
    PROMETHEUS_HANDLE
        .set(handle)
        .map_err(|_| MetricsError::PrometheusErr("Metrics recorder already initialized".into()))?;

    Ok(())
}

/// Render all metrics recorded via the `metrics` crate in Prometheus text format.
///
/// Returns the metrics as a string suitable for the /metrics endpoint.
pub fn render_metrics() -> String {
    PROMETHEUS_HANDLE
        .get()
        .map(|h| h.render())
        .unwrap_or_default()
}

/// Check if the metrics recorder has been initialized
pub fn is_initialized() -> bool {
    PROMETHEUS_HANDLE.get().is_some()
}
