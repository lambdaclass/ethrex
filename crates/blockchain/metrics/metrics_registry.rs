use prometheus::Registry;

use crate::MetricsError;

const DEFAULT_PREFIX: &str = "ethrex";

/// Builds a Registry that applies: an optional metric-name prefix (env: METRICS_PREFIX)
pub fn registry_with_prefix() -> Result<Registry, MetricsError> {
    let prefix = std::env::var("METRICS_PREFIX").unwrap_or_else(|_| DEFAULT_PREFIX.to_string());

    Registry::new_custom(Some(prefix), None)
        .map_err(|e| MetricsError::PrometheusErr(e.to_string()))
}
