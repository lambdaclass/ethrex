use prometheus::{Encoder, Registry, TextEncoder};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_PROCESS: LazyLock<MetricsProcess> = LazyLock::new(MetricsProcess::default);

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
                        "Failed to register process collector: {e}",
                    ))
                })?;
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
