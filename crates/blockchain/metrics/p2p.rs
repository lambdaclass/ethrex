use metrics::{counter, gauge};
use prometheus::{Encoder, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_P2P: LazyLock<MetricsP2P> = LazyLock::new(MetricsP2P::default);

#[derive(Debug, Clone)]
pub struct MetricsP2P {
    peer_count: IntGauge,
    peer_clients: IntGaugeVec,
    disconnections: IntCounterVec,
}

impl Default for MetricsP2P {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsP2P {
    pub fn new() -> Self {
        MetricsP2P {
            peer_count: IntGauge::new("old_p2p_peer_count", "[DEPRECATED] Current number of connected peers")
                .expect("Failed to create peer_count metric"),
            peer_clients: IntGaugeVec::new(
                Opts::new("old_p2p_peer_clients", "[DEPRECATED] Number of peers by client type"),
                &["client_name"],
            )
            .expect("Failed to create peer_clients metric"),
            disconnections: IntCounterVec::new(
                Opts::new(
                    "old_p2p_disconnections",
                    "[DEPRECATED] Total number of peer disconnections",
                ),
                &["reason", "client_name"],
            )
            .expect("Failed to create disconnections metric"),
        }
    }

    pub fn inc_peer_count(&self) {
        self.peer_count.inc();
        gauge!("p2p_peer_count").increment(1.0);
    }

    pub fn dec_peer_count(&self) {
        self.peer_count.dec();
        gauge!("p2p_peer_count").decrement(1.0);
    }

    pub fn inc_peer_client(&self, client_name: &str) {
        self.peer_clients.with_label_values(&[client_name]).inc();
        gauge!("p2p_peer_clients", "client_name" => client_name.to_string()).increment(1.0);
    }

    pub fn dec_peer_client(&self, client_name: &str) {
        self.peer_clients.with_label_values(&[client_name]).dec();
        gauge!("p2p_peer_clients", "client_name" => client_name.to_string()).decrement(1.0);
    }

    pub fn inc_disconnection(&self, reason: &str, client_name: &str) {
        self.disconnections
            .with_label_values(&[reason, client_name])
            .inc();
        counter!(
            "p2p_disconnections_total",
            "reason" => reason.to_string(),
            "client_name" => client_name.to_string()
        )
        .increment(1);
    }

    pub fn init_disconnection(&self, reason: &str, client_name: &str) {
        self.disconnections
            .with_label_values(&[reason, client_name])
            .inc_by(0);
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        r.register(Box::new(self.peer_count.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.peer_clients.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.disconnections.clone()))
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
