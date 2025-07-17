use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, LazyLock},
    time::{Duration, SystemTime},
};

use prometheus::{Gauge, IntCounter, Registry};
use tokio::sync::Mutex;

use crate::rlpx::{error::RLPxError, p2p::DisconnectReason};

pub static METRICS: LazyLock<Metrics> = LazyLock::new(Metrics::default);

#[derive(Debug, Clone)]
pub struct Metrics {
    _registry: Registry,
    pub window_size: Duration,

    /// Nodes we've contacted over time.
    pub discovered_nodes: IntCounter,
    /// Nodes that successfully answered our ping.
    pub contacts: Arc<Mutex<u64>>,
    pub new_contacts_events: Arc<Mutex<VecDeque<SystemTime>>>,
    /// Nodes we either fail to ping or failed to pong us.
    pub discarded_nodes: IntCounter,
    /// The rate at which we get new contacts
    pub new_contacts_rate: Gauge,

    pub connection_attempts: IntCounter,
    pub connection_attempts_events: Arc<Mutex<VecDeque<SystemTime>>>,
    pub new_connection_attempts_rate: Gauge,

    /// Peers we've connected over time.
    pub connection_establishments: IntCounter,
    pub connection_establishments_events: Arc<Mutex<VecDeque<SystemTime>>>,
    /// The rate at which we get new peers
    pub new_connection_establishments_rate: Gauge,
    /// Peers.
    pub peers: Arc<Mutex<u64>>,
    /// The amount of clients connected grouped by client type
    pub peers_by_client_type: Arc<Mutex<BTreeMap<String, u64>>>,
    /// Ex-peers.
    pub disconnections: Arc<Mutex<BTreeMap<String, u64>>>,
    /// RLPx connection attempt failures grouped and counted by reason
    pub connection_attempt_failures: Arc<Mutex<BTreeMap<String, u64>>>,

    start_time: SystemTime,
}

impl Metrics {
    pub async fn record_new_discovery(&self) {
        let mut events = self.new_contacts_events.lock().await;

        events.push_back(SystemTime::now());

        self.discovered_nodes.inc();

        *self.contacts.lock().await += 1;

        self.update_rate(&mut events, &self.new_contacts_rate).await;
    }

    pub async fn record_new_discarded_node(&self) {
        self.discarded_nodes.inc();

        *self.contacts.lock().await -= 1;
    }

    pub async fn record_new_rlpx_conn_attempt(&self) {
        let mut events = self.connection_attempts_events.lock().await;

        events.push_back(SystemTime::now());

        self.connection_attempts.inc();

        self.update_rate(&mut events, &self.new_connection_attempts_rate)
            .await;
    }

    pub async fn record_new_rlpx_conn_established(&self, client_version: &str) {
        let mut events = self.connection_establishments_events.lock().await;

        events.push_back(SystemTime::now());

        self.connection_establishments.inc();

        *self.peers.lock().await += 1;

        self.update_rate(&mut events, &self.new_connection_establishments_rate)
            .await;

        let mut clients = self.peers_by_client_type.lock().await;
        let split = client_version.split('/').collect::<Vec<&str>>();
        let client_type = split.first().expect("Split always returns 1 element");

        clients
            .entry(client_type.to_string())
            .and_modify(|count| *count += 1)
            .or_insert(1);
    }

    pub async fn record_new_rlpx_conn_disconnection(
        &self,
        client_version: &str,
        reason: DisconnectReason,
    ) {
        *self.peers.lock().await -= 1;

        self.disconnections
            .lock()
            .await
            .entry(reason.to_string())
            .and_modify(|e| *e += 1)
            .or_insert(1);

        let mut clients = self.peers_by_client_type.lock().await;
        let split = client_version.split('/').collect::<Vec<&str>>();
        let client_type = split.first().expect("Split always returns 1 element");

        clients
            .entry(client_type.to_string())
            .and_modify(|count| *count -= 1);
    }

    pub async fn record_new_rlpx_conn_failure(&self, reason: RLPxError) {
        let mut failures_grouped_by_reason = self.connection_attempt_failures.lock().await;

        self.update_failures_grouped_by_reason(&mut failures_grouped_by_reason, &reason)
            .await;
    }

    pub async fn update_rate(&self, events: &mut VecDeque<SystemTime>, rate_gauge: &Gauge) {
        self.clean_old_events(events).await;

        let count = events.len() as f64;

        let windows_size_in_secs = self.window_size.as_secs_f64();

        let elapsed_from_start_time_in_secs =
            self.start_time.elapsed().unwrap_or_default().as_secs_f64();

        let window_secs = if elapsed_from_start_time_in_secs < windows_size_in_secs {
            elapsed_from_start_time_in_secs
        } else {
            windows_size_in_secs
        };

        let rate = if window_secs > 0.0 {
            count / window_secs
        } else {
            0.0
        };

        rate_gauge.set(rate);
    }

    pub async fn clean_old_events(&self, events: &mut VecDeque<SystemTime>) {
        let now = SystemTime::now();

        while let Some(&event_time) = events.front() {
            if now.duration_since(event_time).unwrap_or_default() > self.window_size {
                events.pop_front();
            } else {
                break;
            }
        }
    }

    pub async fn update_failures_grouped_by_reason(
        &self,
        failures_grouped_by_reason: &mut BTreeMap<String, u64>,
        failure_reason: &RLPxError,
    ) {
        match failure_reason {
            RLPxError::HandshakeError(reason) => {
                failures_grouped_by_reason
                    .entry(format!("HandshakeError - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::StateError(reason) => {
                failures_grouped_by_reason
                    .entry(format!("StateError - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::NoMatchingCapabilities() => {
                failures_grouped_by_reason
                    .entry("NoMatchingCapabilities".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::Disconnected() => {
                failures_grouped_by_reason
                    .entry("Disconnected".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::DisconnectReceived(disconnect_reason) => {
                failures_grouped_by_reason
                    .entry(format!("DisconnectReceived - {disconnect_reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::DisconnectSent(disconnect_reason) => {
                failures_grouped_by_reason
                    .entry(format!("DisconnectSent - {disconnect_reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::NotFound(reason) => {
                failures_grouped_by_reason
                    .entry(format!("NotFound - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::InvalidPeerId() => {
                failures_grouped_by_reason
                    .entry("InvalidPeerId".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::InvalidRecoveryId() => {
                failures_grouped_by_reason
                    .entry("InvalidRecoveryId".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::InvalidMessageLength() => {
                failures_grouped_by_reason
                    .entry("InvalidMessageLength".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::MessageNotHandled(reason) => {
                failures_grouped_by_reason
                    .entry(format!("MessageNotHandled - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::BadRequest(reason) => {
                failures_grouped_by_reason
                    .entry(format!("BadRequest - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::RLPDecodeError(rlpdecode_error) => {
                failures_grouped_by_reason
                    .entry(format!("RLPDecodeError - {rlpdecode_error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::RLPEncodeError(rlpencode_error) => {
                failures_grouped_by_reason
                    .entry(format!("RLPEncodeError - {rlpencode_error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::StoreError(store_error) => {
                failures_grouped_by_reason
                    .entry(format!("StoreError - {store_error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::CryptographyError(reason) => {
                failures_grouped_by_reason
                    .entry(format!("CryptographyError - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::BroadcastError(reason) => {
                failures_grouped_by_reason
                    .entry(format!("BroadcastError - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::RecvError(recv_error) => {
                failures_grouped_by_reason
                    .entry(format!("RecvError - {recv_error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::SendMessage(reason) => {
                failures_grouped_by_reason
                    .entry(format!("SendMessage - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::MempoolError(mempool_error) => {
                failures_grouped_by_reason
                    .entry(format!("MempoolError - {mempool_error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::IoError(error) => {
                failures_grouped_by_reason
                    .entry(format!("IoError - {error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::InvalidMessageFrame(reason) => {
                failures_grouped_by_reason
                    .entry(format!("InvalidMessageFrame - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::IncompatibleProtocol => {
                failures_grouped_by_reason
                    .entry("IncompatibleProtocol".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::InvalidBlockRange => {
                failures_grouped_by_reason
                    .entry("InvalidBlockRange".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        let registry = Registry::new();

        let discovered_nodes = IntCounter::new(
            "discv4_discovered_nodes",
            "Total number of new nodes discovered",
        )
        .expect("Failed to create discovered_nodes counter");

        let new_contacts_rate = Gauge::new(
            "discv4_new_contacts_rate",
            "Rate of new nodes discovered per second",
        )
        .expect("Failed to create new_contacts_rate gauge");

        let discarded_nodes =
            IntCounter::new("discv4_discarded_nodes", "Total number of discarded nodes")
                .expect("Failed to create discarded_nodes counter");

        registry
            .register(Box::new(discovered_nodes.clone()))
            .expect("Failed to register discovered_nodes counter");

        registry
            .register(Box::new(new_contacts_rate.clone()))
            .expect("Failed to register contacts_rate gauge");

        registry
            .register(Box::new(discarded_nodes.clone()))
            .expect("Failed to register discarded_nodes counter");

        let attempted_rlpx_conn = IntCounter::new(
            "rlpx_attempted_rlpx_conn",
            "Total number of attempted RLPx connections",
        )
        .expect("Failed to create attempted_rlpx_conn counter");

        let attempted_rlpx_conn_rate = Gauge::new(
            "rlpx_attempted_rlpx_conn_rate",
            "Rate of attempted RLPx connections per second",
        )
        .expect("Failed to create attempted_rlpx_conn_rate gauge");

        let established_rlpx_conn = IntCounter::new(
            "rlpx_established_rlpx_conn",
            "Total number of established RLPx connections",
        )
        .expect("Failed to create established_rlpx_conn counter");

        let established_rlpx_conn_rate = Gauge::new(
            "rlpx_established_rlpx_conn_rate",
            "Rate of established RLPx connections per second",
        )
        .expect("Failed to create established_rlpx_conn_rate gauge");

        registry
            .register(Box::new(attempted_rlpx_conn.clone()))
            .expect("Failed to register attempted_rlpx_conn counter");

        registry
            .register(Box::new(attempted_rlpx_conn_rate.clone()))
            .expect("Failed to register attempted_rlpx_conn_rate gauge");

        registry
            .register(Box::new(established_rlpx_conn.clone()))
            .expect("Failed to register established_rlpx_conn counter");

        registry
            .register(Box::new(established_rlpx_conn_rate.clone()))
            .expect("Failed to register established_rlpx_conn_rate gauge");

        Metrics {
            _registry: registry,
            new_contacts_events: Arc::new(Mutex::new(VecDeque::new())),
            window_size: Duration::from_secs(60),

            discovered_nodes,
            contacts: Arc::new(Mutex::new(0)),
            new_contacts_rate,
            discarded_nodes,

            connection_attempts: attempted_rlpx_conn,
            connection_attempts_events: Arc::new(Mutex::new(VecDeque::new())),
            new_connection_attempts_rate: attempted_rlpx_conn_rate,

            connection_establishments: established_rlpx_conn,
            connection_establishments_events: Arc::new(Mutex::new(VecDeque::new())),
            new_connection_establishments_rate: established_rlpx_conn_rate,

            peers: Arc::new(Mutex::new(0)),
            peers_by_client_type: Arc::new(Mutex::new(BTreeMap::new())),

            disconnections: Arc::new(Mutex::new(BTreeMap::new())),

            connection_attempt_failures: Arc::new(Mutex::new(BTreeMap::new())),

            start_time: SystemTime::now(),
        }
    }
}
