use prometheus::{Encoder, IntGaugeVec, Opts, Registry, TextEncoder};
use std::sync::OnceLock;

use crate::MetricsError;

pub static METRICS_NODE: OnceLock<MetricsNode> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct MetricsNode {
    info: IntGaugeVec,
}

impl MetricsNode {
    /// Initialize the node metrics with version information.
    /// This should be called once at startup from the main entry point.
    pub fn init(
        version: &str,
        commit: &str,
        branch: &str,
        rust_version: &str,
        target: &str,
        network: &str,
    ) {
        let info = IntGaugeVec::new(
            Opts::new(
                "ethrex_info",
                "Node information including version and build details",
            ),
            &[
                "version",
                "commit",
                "branch",
                "rust_version",
                "target",
                "network",
            ],
        )
        .expect("Failed to create ethrex_info metric");

        // Set the gauge to 1 with the version labels
        info.with_label_values(&[version, commit, branch, rust_version, target, network])
            .set(1);

        // Ignore error if already initialized
        let _ = METRICS_NODE.set(MetricsNode { info });
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        r.register(Box::new(self.info.clone()))
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
