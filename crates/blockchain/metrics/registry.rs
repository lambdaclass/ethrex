use prometheus::{Encoder, TextEncoder};

use crate::MetricsError;

/// Returns all metrics currently registered in Prometheus' default registry.
///
/// Both profiling and RPC metrics register with this default registry, and the
/// metrics API surfaces them by calling this helper.
pub fn gather_default_metrics() -> Result<String, MetricsError> {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();

    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

    let res = String::from_utf8(buffer)?;

    Ok(res)
}
