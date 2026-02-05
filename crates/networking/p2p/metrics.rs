use core::fmt;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use ethrex_common::H256;
use prometheus::{Gauge, IntCounter, Registry};
use tokio::sync::Mutex;

use crate::rlpx::{error::PeerConnectionError, p2p::DisconnectReason};

pub static METRICS: LazyLock<Metrics> = LazyLock::new(Metrics::default);

#[derive(Debug)]
pub struct Metrics {
    _registry: Registry,
    pub window_size: Duration,
    pub enabled: Arc<Mutex<bool>>,

    /// Nodes we've contacted over time.
    pub discovered_nodes: IntCounter,
    /// Nodes that successfully answered our ping.
    pub contacts: AtomicU64,
    pub new_contacts_events: Arc<Mutex<VecDeque<SystemTime>>>,
    /// Nodes we either fail to ping or failed to pong us.
    pub discarded_nodes: IntCounter,
    /// The rate at which we get new contacts
    pub new_contacts_rate: Gauge,

    pub connection_attempts: IntCounter,
    pub connection_attempts_events: Arc<Mutex<VecDeque<SystemTime>>>,
    pub new_connection_attempts_rate: Gauge,

    pub pings_sent: IntCounter,
    pub pings_sent_events: Arc<Mutex<VecDeque<SystemTime>>>,
    pub pings_sent_rate: Gauge,

    /// Peers we've connected over time.
    pub connection_establishments: IntCounter,
    pub connection_establishments_events: Arc<Mutex<VecDeque<SystemTime>>>,
    /// The rate at which we get new peers
    pub new_connection_establishments_rate: Gauge,
    /// Peers.
    pub peers: AtomicU64,
    /// The amount of clients connected grouped by client type
    pub peers_by_client_type: Arc<Mutex<BTreeMap<String, u64>>>,
    /// Ex-peers by client type and then reason of disconnection.
    pub disconnections_by_client_type: Arc<Mutex<BTreeMap<String, BTreeMap<String, u64>>>>,
    /// RLPx connection attempt failures grouped and counted by reason
    pub connection_attempt_failures: Arc<Mutex<BTreeMap<String, u64>>>,

    /* Snap Sync */
    // Common
    pub sync_head_block: AtomicU64,
    pub sync_head_hash: Arc<Mutex<H256>>,
    pub current_step: Arc<CurrentStep>,

    // Headers
    pub downloaded_headers: IntCounter,
    pub time_to_retrieve_sync_head_block: Arc<Mutex<Option<Duration>>>,
    pub headers_download_start_time: Arc<Mutex<Option<SystemTime>>>,

    // Account tries
    pub downloaded_account_tries: AtomicU64,
    pub account_tries_inserted: AtomicU64,
    pub account_tries_download_start_time: Arc<Mutex<Option<SystemTime>>>,
    pub account_tries_download_end_time: Arc<Mutex<Option<SystemTime>>>,
    pub account_tries_insert_start_time: Arc<Mutex<Option<SystemTime>>>,
    pub account_tries_insert_end_time: Arc<Mutex<Option<SystemTime>>>,

    // Storage tries
    pub storage_tries_download_start_time: Arc<Mutex<Option<SystemTime>>>,
    pub storage_tries_download_end_time: Arc<Mutex<Option<SystemTime>>>,

    // Storage slots
    pub storage_leaves_downloaded: IntCounter,
    pub storage_leaves_inserted: IntCounter,
    pub storage_tries_insert_end_time: Arc<Mutex<Option<SystemTime>>>,
    pub storage_tries_insert_start_time: Arc<Mutex<Option<SystemTime>>>,

    // Healing
    pub healing_empty_try_recv: AtomicU64,
    pub global_state_trie_leafs_healed: AtomicU64,
    pub global_storage_tries_leafs_healed: AtomicU64,
    pub heal_end_time: Arc<Mutex<Option<SystemTime>>>,
    pub heal_start_time: Arc<Mutex<Option<SystemTime>>>,

    // Bytecodes
    pub bytecodes_to_download: AtomicU64,
    pub downloaded_bytecodes: AtomicU64,
    pub bytecode_download_start_time: Arc<Mutex<Option<SystemTime>>>,
    pub bytecode_download_end_time: Arc<Mutex<Option<SystemTime>>>,

    start_time: SystemTime,
}

#[derive(Debug)]
pub struct CurrentStep(AtomicU8);

impl CurrentStep {
    pub fn set(&self, value: CurrentStepValue) {
        self.0.store(value.into(), Ordering::Relaxed);
    }

    pub fn get(&self) -> CurrentStepValue {
        self.0.load(Ordering::Relaxed).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum CurrentStepValue {
    None,
    HealingStorage,
    HealingState,
    RequestingBytecodes,
    RequestingAccountRanges,
    RequestingStorageRanges,
    DownloadingHeaders,
    InsertingStorageRanges,
    InsertingAccountRanges,
    InsertingAccountRangesNoDb,
}

impl From<u8> for CurrentStepValue {
    fn from(value: u8) -> Self {
        match value {
            1 => Self::HealingStorage,
            2 => Self::HealingState,
            3 => Self::RequestingBytecodes,
            4 => Self::RequestingAccountRanges,
            5 => Self::RequestingStorageRanges,
            6 => Self::DownloadingHeaders,
            7 => Self::InsertingStorageRanges,
            8 => Self::InsertingAccountRanges,
            9 => Self::InsertingAccountRangesNoDb,
            _ => Self::None,
        }
    }
}

impl From<CurrentStepValue> for u8 {
    fn from(value: CurrentStepValue) -> Self {
        match value {
            CurrentStepValue::None => 0,
            CurrentStepValue::HealingStorage => 1,
            CurrentStepValue::HealingState => 2,
            CurrentStepValue::RequestingBytecodes => 3,
            CurrentStepValue::RequestingAccountRanges => 4,
            CurrentStepValue::RequestingStorageRanges => 5,
            CurrentStepValue::DownloadingHeaders => 6,
            CurrentStepValue::InsertingStorageRanges => 7,
            CurrentStepValue::InsertingAccountRanges => 8,
            CurrentStepValue::InsertingAccountRangesNoDb => 9,
        }
    }
}

impl fmt::Display for CurrentStepValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CurrentStepValue::None => write!(f, "Unknown"),
            CurrentStepValue::HealingStorage => write!(f, "Healing Storage"),
            CurrentStepValue::HealingState => write!(f, "Healing State"),
            CurrentStepValue::RequestingBytecodes => write!(f, "Requesting Bytecodes"),
            CurrentStepValue::RequestingAccountRanges => write!(f, "Requesting Account Ranges"),
            CurrentStepValue::RequestingStorageRanges => write!(f, "Requesting Storage Ranges"),
            CurrentStepValue::DownloadingHeaders => write!(f, "Downloading Headers"),
            CurrentStepValue::InsertingStorageRanges => {
                write!(f, "Inserting Storage Ranges - Writing to DB")
            }
            CurrentStepValue::InsertingAccountRanges => {
                write!(f, "Inserting Account Ranges - Writing to DB")
            }
            CurrentStepValue::InsertingAccountRangesNoDb => write!(f, "Inserting Account Ranges"),
        }
    }
}

impl Metrics {
    pub async fn enable(&self) {
        *self.enabled.lock().await = true;
    }

    pub async fn disable(&self) {
        *self.enabled.lock().await = false;
    }

    pub async fn record_new_discovery(&self) {
        let mut events = self.new_contacts_events.lock().await;

        events.push_back(SystemTime::now());

        self.discovered_nodes.inc();

        self.contacts.fetch_add(1, Ordering::Relaxed);

        self.update_rate(&mut events, &self.new_contacts_rate);
    }

    pub fn record_new_discarded_node(&self) {
        self.discarded_nodes.inc();

        self.contacts.fetch_sub(1, Ordering::Relaxed);
    }

    pub async fn record_new_rlpx_conn_attempt(&self) {
        let mut events = self.connection_attempts_events.lock().await;

        events.push_back(SystemTime::now());

        self.connection_attempts.inc();

        self.update_rate(&mut events, &self.new_connection_attempts_rate);
    }

    pub async fn record_new_rlpx_conn_established(&self, client_version: &str) {
        let mut events = self.connection_establishments_events.lock().await;

        events.push_back(SystemTime::now());

        self.connection_establishments.inc();

        self.peers.fetch_add(1, Ordering::Relaxed);

        let client_type = client_version.split('/').next().unwrap_or("unknown");

        // TODO (#4240): This module expose metrics to be used in the snapsync logs, to actually
        // expose them in prometheus we need to call the metrics crate instead, so we do it here.
        // In the future this module will be rewritten as part of the snapsync rewrite and all
        // the metrics calls will be done directly using the metrics crate.
        #[cfg(feature = "metrics")]
        {
            use ethrex_metrics::p2p::METRICS_P2P;
            METRICS_P2P.inc_peer_count();
            METRICS_P2P.inc_peer_client(client_type);
        }

        self.update_rate(&mut events, &self.new_connection_establishments_rate);

        let mut clients = self.peers_by_client_type.lock().await;
        clients
            .entry(client_type.to_string())
            .and_modify(|count| *count += 1)
            .or_insert_with(|| {
                // First time seeing this client type, initialize disconnection metrics
                #[cfg(feature = "metrics")]
                {
                    use ethrex_metrics::p2p::METRICS_P2P;
                    for reason in DisconnectReason::all() {
                        METRICS_P2P.init_disconnection(&reason.to_string(), client_type);
                    }
                }
                1
            });
    }

    pub async fn record_ping_sent(&self) {
        let mut events = self.pings_sent_events.lock().await;

        events.push_back(SystemTime::now());

        self.pings_sent.inc();

        self.update_rate(&mut events, &self.pings_sent_rate);
    }

    pub async fn record_new_rlpx_conn_disconnection(
        &self,
        client_version: &str,
        reason: DisconnectReason,
    ) {
        self.peers.fetch_sub(1, Ordering::Relaxed);

        let client_type = client_version.split('/').next().unwrap_or("unknown");
        #[cfg(feature = "metrics")]
        {
            use ethrex_metrics::p2p::METRICS_P2P;
            METRICS_P2P.dec_peer_count();
            METRICS_P2P.dec_peer_client(client_type);
            METRICS_P2P.inc_disconnection(&reason.to_string(), client_type);
        }

        let mut clients = self.peers_by_client_type.lock().await;

        let mut disconnection_by_client = self.disconnections_by_client_type.lock().await;
        disconnection_by_client
            .entry(client_type.to_string())
            .or_insert(BTreeMap::new())
            .entry(reason.to_string())
            .and_modify(|e| *e += 1)
            .or_insert(1);

        clients
            .entry(client_type.to_string())
            .and_modify(|count| *count -= 1);
    }

    pub async fn record_new_rlpx_conn_failure(&self, reason: PeerConnectionError) {
        let mut failures_grouped_by_reason = self.connection_attempt_failures.lock().await;

        self.update_failures_grouped_by_reason(&mut failures_grouped_by_reason, &reason);
    }

    pub fn update_rate(&self, events: &mut VecDeque<SystemTime>, rate_gauge: &Gauge) {
        self.clean_old_events(events);

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

    pub fn clean_old_events(&self, events: &mut VecDeque<SystemTime>) {
        let now = SystemTime::now();

        while let Some(&event_time) = events.front() {
            if now.duration_since(event_time).unwrap_or_default() > self.window_size {
                events.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn update_failures_grouped_by_reason(
        &self,
        failures_grouped_by_reason: &mut BTreeMap<String, u64>,
        failure_reason: &PeerConnectionError,
    ) {
        match failure_reason {
            PeerConnectionError::HandshakeError(reason) => {
                failures_grouped_by_reason
                    .entry(format!("HandshakeError - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::StateError(reason) => {
                failures_grouped_by_reason
                    .entry(format!("StateError - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::NoMatchingCapabilities(_) => {
                failures_grouped_by_reason
                    .entry("NoMatchingCapabilities".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::TooManyPeers => {
                failures_grouped_by_reason
                    .entry("TooManyPeers".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::Disconnected => {
                failures_grouped_by_reason
                    .entry("Disconnected".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::DisconnectReceived(disconnect_reason) => {
                failures_grouped_by_reason
                    .entry(format!("DisconnectReceived - {disconnect_reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::DisconnectSent(disconnect_reason) => {
                failures_grouped_by_reason
                    .entry(format!("DisconnectSent - {disconnect_reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::NotFound(reason) => {
                failures_grouped_by_reason
                    .entry(format!("NotFound - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::InvalidPeerId(_) => {
                failures_grouped_by_reason
                    .entry("InvalidPeerId".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::InvalidRecoveryId => {
                failures_grouped_by_reason
                    .entry("InvalidRecoveryId".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::InvalidMessageLength(_) => {
                failures_grouped_by_reason
                    .entry("InvalidMessageLength".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::ExpectedRequestId(reason) => {
                failures_grouped_by_reason
                    .entry(format!("ExpectedRequestId - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::MessageNotHandled(reason) => {
                failures_grouped_by_reason
                    .entry(format!("MessageNotHandled - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::BadRequest(reason) => {
                failures_grouped_by_reason
                    .entry(format!("BadRequest - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::RLPDecodeError(rlpdecode_error) => {
                failures_grouped_by_reason
                    .entry(format!("RLPDecodeError - {rlpdecode_error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::RLPEncodeError(rlpencode_error) => {
                failures_grouped_by_reason
                    .entry(format!("RLPEncodeError - {rlpencode_error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::StoreError(store_error) => {
                failures_grouped_by_reason
                    .entry(format!("StoreError - {store_error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::CryptographyError(reason) => {
                failures_grouped_by_reason
                    .entry(format!("CryptographyError - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::BroadcastError(reason) => {
                failures_grouped_by_reason
                    .entry(format!("BroadcastError - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::RecvError(recv_error) => {
                failures_grouped_by_reason
                    .entry(format!("RecvError - {recv_error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::SendMessage(reason) => {
                failures_grouped_by_reason
                    .entry(format!("SendMessage - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::MempoolError(mempool_error) => {
                failures_grouped_by_reason
                    .entry(format!("MempoolError - {mempool_error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::IoError(error) => {
                failures_grouped_by_reason
                    .entry(format!("IoError - {error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::InvalidMessageFrame(reason) => {
                failures_grouped_by_reason
                    .entry(format!("InvalidMessageFrame - {reason}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::IncompatibleProtocol => {
                failures_grouped_by_reason
                    .entry("IncompatibleProtocol".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::InvalidBlockRange => {
                failures_grouped_by_reason
                    .entry("InvalidBlockRange".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            #[cfg(feature = "l2")]
            PeerConnectionError::RollupStoreError(error) => {
                failures_grouped_by_reason
                    .entry(format!("RollupStoreError - {error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::BlockchainError(error) => {
                failures_grouped_by_reason
                    .entry(format!("BlockchainError - {error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::InternalError(error) => {
                failures_grouped_by_reason
                    .entry(format!("InternalError - {error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::L2CapabilityNotNegotiated => {
                failures_grouped_by_reason
                    .entry("L2CapabilityNotNegotiated".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::InvalidBlockRangeUpdate => {
                failures_grouped_by_reason
                    .entry("InvalidBlockRangeUpdate".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::PeerTableError(error) => {
                failures_grouped_by_reason
                    .entry(format!("InternalError - {error}"))
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::Timeout => {
                failures_grouped_by_reason
                    .entry("Timeout".to_owned())
                    .and_modify(|e| *e += 1)
                    .or_insert(1);
            }
            PeerConnectionError::UnexpectedResponse(_, _) => {
                failures_grouped_by_reason
                    .entry("UnexpectedResponse".to_owned())
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

        let pings_sent = IntCounter::new("pings_sent", "Total number of pings sent")
            .expect("Failed to create pings_sent counter");

        let pings_sent_rate = Gauge::new("pings_sent_rate", "Rate of pings sent per second")
            .expect("Failed to create pings_sent_rate gauge");

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
            .register(Box::new(pings_sent.clone()))
            .expect("Failed to register pings_sent counter");

        registry
            .register(Box::new(pings_sent_rate.clone()))
            .expect("Failed to register pings_sent_rate gauge");

        let storage_leaves_inserted = IntCounter::new(
            "storage_leaves_inserted",
            "Total number of storage leaves inserted",
        )
        .expect("Failed to create storage_leaves_inserted counter");

        registry
            .register(Box::new(storage_leaves_inserted.clone()))
            .expect("Failed to register storage_leaves_inserted counter");

        let downloaded_headers = IntCounter::new(
            "downloaded_headers",
            "Total number of headers already download",
        )
        .expect("Failed to create downloaded_headers counter");

        registry
            .register(Box::new(downloaded_headers.clone()))
            .expect("Failed to register downloaded_headers counter");

        let storage_leaves_downloaded = IntCounter::new(
            "storage_leaves_downloaded",
            "Total number of storage leaves downloaded",
        )
        .expect("Failed to create storage_leaves_downloaded counter");

        registry
            .register(Box::new(storage_leaves_downloaded.clone()))
            .expect("Failed to register storage_leaves_downloaded counter");

        Metrics {
            _registry: registry,
            enabled: Arc::new(Mutex::new(false)),
            new_contacts_events: Arc::new(Mutex::new(VecDeque::new())),
            window_size: Duration::from_secs(60),

            discovered_nodes,
            contacts: AtomicU64::new(0),
            new_contacts_rate,
            discarded_nodes,

            connection_attempts: attempted_rlpx_conn,
            connection_attempts_events: Arc::new(Mutex::new(VecDeque::new())),
            new_connection_attempts_rate: attempted_rlpx_conn_rate,

            connection_establishments: established_rlpx_conn,
            connection_establishments_events: Arc::new(Mutex::new(VecDeque::new())),
            new_connection_establishments_rate: established_rlpx_conn_rate,

            pings_sent,
            pings_sent_events: Arc::new(Mutex::new(VecDeque::new())),
            pings_sent_rate,

            peers: AtomicU64::new(0),
            peers_by_client_type: Arc::new(Mutex::new(BTreeMap::new())),

            disconnections_by_client_type: Arc::new(Mutex::new(BTreeMap::new())),

            connection_attempt_failures: Arc::new(Mutex::new(BTreeMap::new())),

            /* Snap Sync */
            // Common
            sync_head_block: AtomicU64::new(0),
            sync_head_hash: Arc::new(Mutex::new(H256::default())),
            current_step: Arc::new(CurrentStep(AtomicU8::new(0))),

            // Headers
            downloaded_headers,
            time_to_retrieve_sync_head_block: Arc::new(Mutex::new(None)),
            headers_download_start_time: Arc::new(Mutex::new(None)),

            // Account tries
            downloaded_account_tries: AtomicU64::new(0),
            account_tries_inserted: AtomicU64::new(0),
            account_tries_download_start_time: Arc::new(Mutex::new(None)),
            account_tries_download_end_time: Arc::new(Mutex::new(None)),
            account_tries_insert_start_time: Arc::new(Mutex::new(None)),
            account_tries_insert_end_time: Arc::new(Mutex::new(None)),

            // Storage tries
            storage_tries_download_start_time: Arc::new(Mutex::new(None)),
            storage_tries_download_end_time: Arc::new(Mutex::new(None)),

            // Storage slots
            storage_leaves_downloaded,

            // Storage tries state roots
            storage_leaves_inserted,
            storage_tries_insert_end_time: Arc::new(Mutex::new(None)),
            storage_tries_insert_start_time: Arc::new(Mutex::new(None)),

            // Healing
            healing_empty_try_recv: AtomicU64::new(1),
            global_state_trie_leafs_healed: AtomicU64::new(0),
            global_storage_tries_leafs_healed: AtomicU64::new(0),
            heal_end_time: Arc::new(Mutex::new(None)),
            heal_start_time: Arc::new(Mutex::new(None)),

            // Bytecodes
            bytecodes_to_download: AtomicU64::new(0),
            downloaded_bytecodes: AtomicU64::new(0),
            bytecode_download_start_time: Arc::new(Mutex::new(None)),
            bytecode_download_end_time: Arc::new(Mutex::new(None)),

            start_time: SystemTime::now(),
        }
    }
}
