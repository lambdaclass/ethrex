use prometheus::{Encoder, IntGauge, Registry, TextEncoder};
use std::sync::{LazyLock, OnceLock};

use crate::MetricsError;

pub static METRICS_PROCESS: LazyLock<MetricsProcess> = LazyLock::new(MetricsProcess::default);
static SIZE_ESTIMATOR: OnceLock<Box<dyn Fn() -> u64 + Send + Sync>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct MetricsProcess;

impl Default for MetricsProcess {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsProcess {
    pub fn new() -> Self {
        MetricsProcess
    }

    /// The Process collector gathers standard process metrics (CPU time, RSS, VSZ, FDs, threads, start_time).
    /// But it only works on Linux. This is an initial implementation.
    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        // Register Prometheus' built-in Linux process metrics
        #[cfg(target_os = "linux")]
        {
            use prometheus::process_collector::ProcessCollector;
            r.register(Box::new(ProcessCollector::for_self()))
                .map_err(|e| {
                    MetricsError::PrometheusErr(format!(
                        "Failed to register process collector: {}",
                        e
                    ))
                })?;
        }

        if let Some(estimator) = SIZE_ESTIMATOR.get() {
            let size = estimator();
            let gauge = IntGauge::new(
                "datadir_size_bytes",
                "Total size in bytes consumed by the configured datadir.",
            )
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
            let clamped = size.min(i64::MAX as u64);
            gauge.set(clamped as i64);
            r.register(Box::new(gauge))
                .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        }

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

/// Sets the size estimator function used to report datadir size metrics.
///
/// The estimator should return the approximate size in bytes of the datadir.
/// This is typically backed by the storage layer's `estimate_disk_size()` method.
pub fn set_size_estimator(estimator: Box<dyn Fn() -> u64 + Send + Sync>) {
    let _ = SIZE_ESTIMATOR.set(estimator);
}
