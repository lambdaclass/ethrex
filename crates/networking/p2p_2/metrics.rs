use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, LazyLock},
    time::{Duration, SystemTime},
};

use prometheus::{Gauge, IntCounter, Registry};
use tokio::sync::Mutex;

use crate::rlpx::error::RLPxError;

pub static METRICS: LazyLock<Metrics> = LazyLock::new(Metrics::default);

#[derive(Debug, Clone)]
pub struct Metrics {
    pub registry: Registry,

    pub new_contacts_events: Arc<Mutex<VecDeque<SystemTime>>>,
    pub window_size: Duration,
    pub total_contacts: IntCounter,
    pub current_contacts: Arc<Mutex<u64>>,
    pub new_contacts_rate: Gauge,
    pub discarded_contacts: IntCounter,

    pub rlpx_conn_attempts: IntCounter,
    pub rlpx_conn_attempts_events: Arc<Mutex<VecDeque<SystemTime>>>,
    pub rlpx_conn_attempts_rate: Gauge,

    pub rlpx_conn_establishments: IntCounter,
    pub rlpx_conn_establishments_events: Arc<Mutex<VecDeque<SystemTime>>>,
    pub rlpx_conn_establishments_rate: Gauge,

    pub rlpx_conn_failures_reasons: Arc<Mutex<Vec<RLPxError>>>,
    pub rlpx_conn_failures_reasons_counts: Arc<Mutex<BTreeMap<String, u64>>>,
    pub rlpx_conn_failures: IntCounter,

    start_time: SystemTime,
}

impl Metrics {
    pub fn new(window_size_in_secs: u64) -> Self {
        Metrics {
            window_size: Duration::from_secs(window_size_in_secs),
            ..Default::default()
        }
    }

    pub async fn record_new_contact(&self) {
        let mut events = self.new_contacts_events.lock().await;
        events.push_back(SystemTime::now());
        self.total_contacts.inc();
        self.increase_current_contacts().await;
        self.update_rate(&mut events, &self.new_contacts_rate).await;
    }

    pub async fn record_discarded_contact(&self) {
        self.discarded_contacts.inc();
        self.decrease_current_contacts().await;
    }

    pub async fn record_new_rlpx_conn_attempt(&self) {
        let mut events = self.rlpx_conn_attempts_events.lock().await;
        events.push_back(SystemTime::now());
        self.rlpx_conn_attempts.inc();
        self.update_rate(&mut events, &self.rlpx_conn_attempts_rate)
            .await;
    }

    pub async fn record_new_rlpx_conn_established(&self) {
        let mut events = self.rlpx_conn_establishments_events.lock().await;
        events.push_back(SystemTime::now());
        self.rlpx_conn_establishments.inc();
        self.update_rate(&mut events, &self.rlpx_conn_establishments_rate)
            .await;
    }

    pub async fn record_new_rlpx_conn_failure(&self, reason: RLPxError) {
        let mut events = self.rlpx_conn_establishments_events.lock().await;
        events.push_back(SystemTime::now());
        self.rlpx_conn_failures.inc();
        self.update_rlpx_conn_failures_counts(&reason).await;
        self.rlpx_conn_failures_reasons.lock().await.push(reason);
    }

    pub async fn increase_current_contacts(&self) {
        *self.current_contacts.lock().await += 1
    }

    pub async fn decrease_current_contacts(&self) {
        *self.current_contacts.lock().await -= 1
    }

    pub async fn update_rlpx_conn_failures_counts(&self, failure_reason: &RLPxError) {
        let mut failures_grouped_by_reason = self.rlpx_conn_failures_reasons_counts.lock().await;

        match failure_reason {
            RLPxError::HandshakeError(reason) if reason == "Network Id does not match" => {
                failures_grouped_by_reason
                    .entry("HandshakeError - Chain ID Mismatch".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::HandshakeError(reason)
                if reason == "Eth protocol version does not match" =>
            {
                failures_grouped_by_reason
                    .entry("HandshakeError - Eth Protocol Version Mismatch".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::HandshakeError(reason) if reason == "Genesis does not match" => {
                failures_grouped_by_reason
                    .entry("HandshakeError - Genesis Mismatch".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::HandshakeError(reason) if reason == "Invalid Fork Id" => {
                failures_grouped_by_reason
                    .entry("HandshakeError - Fork ID Mismatch".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::HandshakeError(_) => {
                failures_grouped_by_reason
                    .entry("HandshakeError".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::StateError(_) => {
                failures_grouped_by_reason
                    .entry("StateError".to_owned())
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
            RLPxError::NotFound(_) => {
                failures_grouped_by_reason
                    .entry("NotFound".to_owned())
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
            RLPxError::MessageNotHandled(_) => {
                failures_grouped_by_reason
                    .entry("MessageNotHandled".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::BadRequest(_) => {
                failures_grouped_by_reason
                    .entry("BadRequest".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::RLPDecodeError(rlpdecode_error) => {
                failures_grouped_by_reason
                    .entry("RLPDecodeError".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::RLPEncodeError(rlpencode_error) => {
                failures_grouped_by_reason
                    .entry("RLPEncodeError".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::StoreError(store_error) => {
                failures_grouped_by_reason
                    .entry("StoreError".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::CryptographyError(_) => {
                failures_grouped_by_reason
                    .entry("CryptographyError".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::BroadcastError(_) => {
                failures_grouped_by_reason
                    .entry("BroadcastError".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::RecvError(recv_error) => {
                failures_grouped_by_reason
                    .entry("RecvError".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::SendMessage(_) => {
                failures_grouped_by_reason
                    .entry("SendMessage".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::MempoolError(mempool_error) => {
                failures_grouped_by_reason
                    .entry("MempoolError".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::IoError(error) => {
                failures_grouped_by_reason
                    .entry(format!("IoError - {error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            RLPxError::InvalidMessageFrame(_) => {
                failures_grouped_by_reason
                    .entry("InvalidMessageFrame".to_owned())
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
}

impl Default for Metrics {
    fn default() -> Self {
        let registry = Registry::new();

        let new_contacts_total = IntCounter::new(
            "discv4_new_contacts_total",
            "Total number of new nodes discovered",
        )
        .expect("Failed to create new_contacts_total counter");

        let contacts_rate = Gauge::new(
            "discv4_contacts_rate",
            "Rate of new nodes discovered per second",
        )
        .expect("Failed to create contacts_rate gauge");

        let discarded_contacts = IntCounter::new(
            "discv4_discarded_contacts",
            "Total number of discarded nodes",
        )
        .expect("Failed to create discarded_contacts counter");

        registry
            .register(Box::new(new_contacts_total.clone()))
            .expect("Failed to register new_contacts_total counter");

        registry
            .register(Box::new(contacts_rate.clone()))
            .expect("Failed to register contacts_rate gauge");

        registry
            .register(Box::new(discarded_contacts.clone()))
            .expect("Failed to register discarded_contacts counter");

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

        let failed_rlpx_conn = IntCounter::new(
            "rlpx_failed_rlpx_conn",
            "Total number of failed RLPx connections",
        )
        .expect("Failed to create failed_rlpx_conn counter");

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

        registry
            .register(Box::new(failed_rlpx_conn.clone()))
            .expect("Failed to register failed_rlpx_conn counter");

        Metrics {
            registry,
            new_contacts_events: Arc::new(Mutex::new(VecDeque::new())),
            window_size: Duration::from_secs(60),
            total_contacts: new_contacts_total,
            current_contacts: Arc::new(Mutex::new(0)),
            new_contacts_rate: contacts_rate,
            discarded_contacts,
            rlpx_conn_attempts: attempted_rlpx_conn,
            rlpx_conn_attempts_events: Arc::new(Mutex::new(VecDeque::new())),
            rlpx_conn_attempts_rate: attempted_rlpx_conn_rate,
            rlpx_conn_establishments: established_rlpx_conn,
            rlpx_conn_establishments_events: Arc::new(Mutex::new(VecDeque::new())),
            rlpx_conn_establishments_rate: established_rlpx_conn_rate,
            rlpx_conn_failures: failed_rlpx_conn,
            rlpx_conn_failures_reasons: Arc::new(Mutex::new(Vec::new())),
            rlpx_conn_failures_reasons_counts: Arc::new(Mutex::new(BTreeMap::new())),
            start_time: SystemTime::now(),
        }
    }
}
