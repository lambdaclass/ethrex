use prometheus::{Encoder, Gauge, IntGauge, Registry, TextEncoder};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_SNAP_SYNC: LazyLock<MetricsSnapSync> = LazyLock::new(MetricsSnapSync::default);

#[derive(Debug, Clone)]
pub struct MetricsSnapSync {
    duration_seconds: Gauge,
    current_step: IntGauge,
}

impl Default for MetricsSnapSync {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsSnapSync {
    pub fn new() -> Self {
        let duration_seconds = Gauge::new(
            "snap_sync_duration_seconds",
            "Elapsed snap sync time in seconds",
        )
        .expect("Failed to create snap_sync_duration_seconds gauge");

        let current_step = IntGauge::new(
            "snap_sync_current_step",
            "Snap sync current step encoded as an ordinal value",
        )
        .expect("Failed to create snap_sync_current_step gauge");

        MetricsSnapSync {
            duration_seconds,
            current_step,
        }
    }

    pub fn set_duration_seconds(&self, duration_seconds: f64) {
        self.duration_seconds.set(duration_seconds);
    }

    pub fn set_current_step(&self, current_step: u8) {
        self.current_step.set(current_step.into());
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        if self.current_step.get() <= 0 && self.duration_seconds.get() <= 0.0 {
            return Ok(String::new());
        }

        let registry = Registry::new();

        registry
            .register(Box::new(self.duration_seconds.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        registry
            .register(Box::new(self.current_step.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        let metric_families = registry.gather();
        let mut buffer = Vec::new();
        TextEncoder::new()
            .encode(&metric_families, &mut buffer)
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        Ok(String::from_utf8(buffer)?)
    }
}
