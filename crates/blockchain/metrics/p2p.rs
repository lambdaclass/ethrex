use prometheus::{Encoder, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, LazyLock, Mutex},
};

use crate::MetricsError;

pub static METRICS_P2P: LazyLock<MetricsP2P> = LazyLock::new(MetricsP2P::default);

#[derive(Debug, Clone)]
pub struct MetricsP2P {
    peer_count: IntGauge,
    peer_clients: IntGaugeVec,
    disconnections: IntCounterVec,
    known_clients: Arc<Mutex<HashSet<String>>>,
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
            known_clients: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn update_peers(&self, active_peers: i64, current_clients: HashMap<String, i64>) {
        self.peer_count.set(active_peers);

        let mut known_clients = self.known_clients.lock().unwrap();

        // Update metrics for current clients
        for (client, count) in &current_clients {
            self.peer_clients.with_label_values(&[client]).set(*count);
            known_clients.insert(client.clone());
        }

        // Zero out clients that are no longer present
        let mut clients_to_remove = Vec::new();
        for client in known_clients.iter() {
            if !current_clients.contains_key(client) {
                self.peer_clients.with_label_values(&[client]).set(0);
                clients_to_remove.push(client.clone());
            }
        }
        for client in clients_to_remove {
            known_clients.remove(&client);
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
