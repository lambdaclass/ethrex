use prometheus::{Encoder, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder};
use std::sync::LazyLock;

use crate::MetricsError;

pub static METRICS_P2P: LazyLock<MetricsP2P> = LazyLock::new(MetricsP2P::default);

#[derive(Debug, Clone)]
pub struct MetricsP2P {
    peer_count: IntGauge,
    peer_clients: IntGaugeVec,
    disconnections: IntCounterVec,
    incoming_messages: IntCounterVec,
    outgoing_messages: IntCounterVec,
    discv4_incoming_messages: IntCounterVec,
    discv4_outgoing_messages: IntCounterVec,
    discv5_incoming_messages: IntCounterVec,
    discv5_outgoing_messages: IntCounterVec,
}

impl Default for MetricsP2P {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsP2P {
    pub fn new() -> Self {
        MetricsP2P {
            peer_count: IntGauge::new("ethrex_p2p_peer_count", "Current number of connected peers")
                .expect("Failed to create peer_count metric"),
            peer_clients: IntGaugeVec::new(
                Opts::new("ethrex_p2p_peer_clients", "Number of peers by client type"),
                &["client_name"],
            )
            .expect("Failed to create peer_clients metric"),
            disconnections: IntCounterVec::new(
                Opts::new(
                    "ethrex_p2p_disconnections",
                    "Total number of peer disconnections",
                ),
                &["reason", "client_name"],
            )
            .expect("Failed to create disconnections metric"),
            incoming_messages: IntCounterVec::new(
                Opts::new(
                    "ethrex_p2p_incoming_messages",
                    "Total number of incoming P2P messages by type",
                ),
                &["msg_type"],
            )
            .expect("Failed to create incoming_messages metric"),
            outgoing_messages: IntCounterVec::new(
                Opts::new(
                    "ethrex_p2p_outgoing_messages",
                    "Total number of outgoing P2P messages by type",
                ),
                &["msg_type"],
            )
            .expect("Failed to create outgoing_messages metric"),
            discv4_incoming_messages: IntCounterVec::new(
                Opts::new(
                    "ethrex_discv4_incoming_messages",
                    "Total number of incoming discv4 discovery messages by type",
                ),
                &["msg_type"],
            )
            .expect("Failed to create discv4_incoming_messages metric"),
            discv4_outgoing_messages: IntCounterVec::new(
                Opts::new(
                    "ethrex_discv4_outgoing_messages",
                    "Total number of outgoing discv4 discovery messages by type",
                ),
                &["msg_type"],
            )
            .expect("Failed to create discv4_outgoing_messages metric"),
            discv5_incoming_messages: IntCounterVec::new(
                Opts::new(
                    "ethrex_discv5_incoming_messages",
                    "Total number of incoming discv5 discovery messages by type",
                ),
                &["msg_type"],
            )
            .expect("Failed to create discv5_incoming_messages metric"),
            discv5_outgoing_messages: IntCounterVec::new(
                Opts::new(
                    "ethrex_discv5_outgoing_messages",
                    "Total number of outgoing discv5 discovery messages by type",
                ),
                &["msg_type"],
            )
            .expect("Failed to create discv5_outgoing_messages metric"),
        }
    }

    pub fn inc_peer_count(&self) {
        self.peer_count.inc();
    }

    pub fn dec_peer_count(&self) {
        self.peer_count.dec();
    }

    pub fn inc_peer_client(&self, client_name: &str) {
        self.peer_clients.with_label_values(&[client_name]).inc();
    }

    pub fn dec_peer_client(&self, client_name: &str) {
        self.peer_clients.with_label_values(&[client_name]).dec();
    }

    pub fn inc_disconnection(&self, reason: &str, client_name: &str) {
        self.disconnections
            .with_label_values(&[reason, client_name])
            .inc();
    }

    pub fn init_disconnection(&self, reason: &str, client_name: &str) {
        self.disconnections
            .with_label_values(&[reason, client_name])
            .inc_by(0);
    }

    pub fn inc_incoming_message(&self, msg_type: &str) {
        self.incoming_messages.with_label_values(&[msg_type]).inc();
    }

    pub fn inc_outgoing_message(&self, msg_type: &str) {
        self.outgoing_messages.with_label_values(&[msg_type]).inc();
    }

    pub fn inc_discv4_incoming(&self, msg_type: &str) {
        self.discv4_incoming_messages
            .with_label_values(&[msg_type])
            .inc();
    }

    pub fn inc_discv4_outgoing(&self, msg_type: &str) {
        self.discv4_outgoing_messages
            .with_label_values(&[msg_type])
            .inc();
    }

    pub fn inc_discv5_incoming(&self, msg_type: &str) {
        self.discv5_incoming_messages
            .with_label_values(&[msg_type])
            .inc();
    }

    pub fn inc_discv5_outgoing(&self, msg_type: &str) {
        self.discv5_outgoing_messages
            .with_label_values(&[msg_type])
            .inc();
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        r.register(Box::new(self.peer_count.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.peer_clients.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.disconnections.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.incoming_messages.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.outgoing_messages.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.discv4_incoming_messages.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.discv4_outgoing_messages.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.discv5_incoming_messages.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        r.register(Box::new(self.discv5_outgoing_messages.clone()))
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
