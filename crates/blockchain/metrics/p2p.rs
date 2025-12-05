use prometheus::{
    Encoder, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder,
    register_histogram_vec,
};
use std::{future::Future, sync::LazyLock};

use crate::MetricsError;

pub static METRICS_P2P: LazyLock<MetricsP2P> = LazyLock::new(MetricsP2P::default);

/// Histogram for P2P message handling duration.
/// Registered in the default Prometheus registry so it's automatically gathered.
pub static METRICS_P2P_MESSAGE_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "ethrex_p2p_message_duration_seconds",
        "Histogram of P2P message handling duration partitioned by message type",
        &["message_type"],
    )
    .expect("Failed to create p2p message duration histogram")
});

/// Records the duration of an async P2P message handler.
///
/// Use this at the centralized message dispatch point to automatically
/// track latency for all P2P message types.
pub async fn record_p2p_message_duration<Fut, T>(message_type: &str, future: Fut) -> T
where
    Fut: Future<Output = T>,
{
    let timer = METRICS_P2P_MESSAGE_DURATION
        .with_label_values(&[message_type])
        .start_timer();

    let output = future.await;
    timer.observe_duration();
    output
}

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
