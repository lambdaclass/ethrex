use crate::{
    discv4::{
        server::{DiscoveryServer, DiscoveryServerError},
        side_car::{DiscoverySideCar, DiscoverySideCarError},
    },
    kademlia::{Kademlia, PeerData},
    metrics::METRICS,
    rlpx::{
        connection::server::{RLPxConnBroadcastSender, RLPxConnection},
        initiator::{RLPxInitiator, RLPxInitiatorError},
        l2::l2_connection::P2PBasedContext,
        message::Message,
        p2p::SUPPORTED_SNAP_CAPABILITIES,
    },
    tx_broadcaster::{TxBroadcaster, TxBroadcasterError},
    types::{Node, NodeRecord},
};
use ethrex_blockchain::Blockchain;
use ethrex_common::H256;
use ethrex_storage::Store;
use secp256k1::SecretKey;
use std::{
    collections::BTreeMap,
    io,
    net::SocketAddr,
    str::FromStr,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::{
    net::{TcpListener, TcpSocket, UdpSocket},
    sync::Mutex,
};
use tokio_util::task::TaskTracker;
use tracing::{debug, error, info};

pub const MAX_MESSAGES_TO_BROADCAST: usize = 100000;

#[derive(Clone, Debug)]
pub struct P2PContext {
    pub tracker: TaskTracker,
    pub signer: SecretKey,
    pub table: Kademlia,
    pub storage: Store,
    pub blockchain: Arc<Blockchain>,
    pub(crate) broadcast: RLPxConnBroadcastSender,
    pub local_node: Node,
    pub local_node_record: Arc<Mutex<NodeRecord>>,
    pub client_version: String,
    pub based_context: Option<P2PBasedContext>,
}

impl P2PContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        local_node: Node,
        local_node_record: Arc<Mutex<NodeRecord>>,
        tracker: TaskTracker,
        signer: SecretKey,
        peer_table: Kademlia,
        storage: Store,
        blockchain: Arc<Blockchain>,
        client_version: String,
        based_context: Option<P2PBasedContext>,
    ) -> Self {
        let (channel_broadcast_send_end, _) = tokio::sync::broadcast::channel::<(
            tokio::task::Id,
            Arc<Message>,
        )>(MAX_MESSAGES_TO_BROADCAST);

        P2PContext {
            local_node,
            local_node_record,
            tracker,
            signer,
            table: peer_table,
            storage,
            blockchain,
            broadcast: channel_broadcast_send_end,
            client_version,
            based_context,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("Failed to start discovery server: {0}")]
    DiscoveryServerError(#[from] DiscoveryServerError),
    #[error("Failed to start discovery side car: {0}")]
    DiscoverySideCarError(#[from] DiscoverySideCarError),
    #[error("Failed to start RLPx Initiator: {0}")]
    RLPxInitiatorError(#[from] RLPxInitiatorError),
    #[error("Failed to start Tx Broadcaster: {0}")]
    TxBroadcasterError(#[from] TxBroadcasterError),
}

pub fn peer_table() -> Kademlia {
    Kademlia::new()
}

pub async fn start_network(context: P2PContext, bootnodes: Vec<Node>) -> Result<(), NetworkError> {
    let udp4_socket = Arc::new(
        UdpSocket::bind(context.local_node.udp_addr())
            .await
            .expect("Failed to bind udp socket"),
    );
    let ip6_addr = SocketAddr::from_str(&format!(
        "[::ffff:{}]:{}",
        context.local_node.ip, context.local_node.udp_port
    ))
    .expect("Failed to parse address");
    let udp6_socket = Arc::new(
        UdpSocket::bind(ip6_addr)
            .await
            .expect("Failed to bind udp socket"),
    );

    DiscoveryServer::spawn(
        context.local_node.clone(),
        context.signer,
        udp4_socket.clone(),
        udp6_socket.clone(),
        context.table.clone(),
        bootnodes,
    )
    .await
    .inspect_err(|e| {
        error!("Failed to start discovery server: {e}");
    })?;

    DiscoverySideCar::spawn(
        context.local_node.clone(),
        context.signer,
        udp4_socket,
        udp6_socket,
        context.table.clone(),
    )
    .await
    .inspect_err(|e| {
        error!("Failed to start discovery side car: {e}");
    })?;

    RLPxInitiator::spawn(context.clone())
        .await
        .inspect_err(|e| {
            error!("Failed to start RLPx Initiator: {e}");
        })?;

    TxBroadcaster::spawn(context.table.clone(), context.blockchain.clone())
        .await
        .inspect_err(|e| {
            error!("Failed to start Tx Broadcaster: {e}");
        })?;

    context.tracker.spawn(serve_p2p_requests(context.clone()));

    Ok(())
}

pub(crate) async fn serve_p2p_requests(context: P2PContext) {
    let tcp_addr = context.local_node.tcp_addr();
    let listener = match listener(tcp_addr) {
        Ok(result) => result,
        Err(e) => {
            error!("Error opening tcp socket at {tcp_addr}: {e}. Stopping p2p server");
            return;
        }
    };
    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(result) => result,
            Err(e) => {
                error!("Error receiving data from tcp socket {tcp_addr}: {e}. Stopping p2p server");
                return;
            }
        };

        if tcp_addr == peer_addr {
            // Ignore connections from self
            continue;
        }

        let _ = RLPxConnection::spawn_as_receiver(context.clone(), peer_addr, stream).await;
    }
}

fn listener(tcp_addr: SocketAddr) -> Result<TcpListener, io::Error> {
    let tcp_socket = TcpSocket::new_v4()?;
    tcp_socket.bind(tcp_addr)?;
    tcp_socket.listen(50)
}

pub async fn periodically_show_peer_stats(
    blockchain: Arc<Blockchain>,
    peers: Arc<Mutex<BTreeMap<H256, PeerData>>>,
) {
    periodically_show_peer_stats_during_syncing(blockchain).await;
    periodically_show_peer_stats_after_sync(peers).await;
}

pub async fn periodically_show_peer_stats_during_syncing(blockchain: Arc<Blockchain>) {
    let start = std::time::Instant::now();
    loop {
        if blockchain.is_synced() {
            return;
        }
        let metrics_enabled = *METRICS.enabled.lock().await;
        // Show the metrics only when these are enabled
        if !metrics_enabled {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        let rlpx_connection_failures = METRICS.connection_attempt_failures.lock().await;

        let rlpx_connection_client_types = METRICS.peers_by_client_type.lock().await;

        let rlpx_disconnections = METRICS.disconnections_by_client_type.lock().await;

        /* Snap Sync */

        let total_headers_to_download = METRICS.headers_to_download.lock().await;
        let downloaded_headers = METRICS.downloaded_headers.lock().await;
        let remaining_headers = total_headers_to_download.saturating_sub(*downloaded_headers);

        let current_headers_download_progress = if *total_headers_to_download == 0 {
            0.0
        } else {
            (*downloaded_headers as f64 / *total_headers_to_download as f64) * 100.0
        };

        let mut maybe_headers_download_time =
            METRICS.headers_download_start_time.lock().await.map(|t| {
                t.elapsed()
                    .expect("Failed to get current headers download time")
            });

        let mut maybe_time_taken_to_download_headers =
            METRICS.time_taken_to_download_headers.lock().await;

        if remaining_headers == 0 {
            if maybe_time_taken_to_download_headers.is_none() {
                *maybe_time_taken_to_download_headers = maybe_headers_download_time;
            } else {
                maybe_headers_download_time = *maybe_time_taken_to_download_headers;
            }
        }

        let downloaded_account_tries = *METRICS.downloaded_account_tries.lock().await;

        let time_taken_to_download_account_tries = {
            let end_time = METRICS
                .account_tries_download_end_time
                .lock()
                .await
                .unwrap_or(SystemTime::now());

            METRICS
                .account_tries_download_start_time
                .lock()
                .await
                .map(|start_time| {
                    end_time
                        .duration_since(start_time)
                        .unwrap_or(Duration::from_secs(0))
                })
        };

        let time_taken_to_download_storage_tries = {
            let end_time = METRICS
                .storage_tries_download_end_time
                .lock()
                .await
                .unwrap_or(SystemTime::now());

            METRICS
                .storage_tries_download_start_time
                .lock()
                .await
                .map(|start_time| {
                    end_time
                        .duration_since(start_time)
                        .unwrap_or(Duration::from_secs(0))
                })
        };

        let total_storage_tries_to_download = METRICS.storage_tries_to_download.lock().await;

        let downloaded_storage_tries = METRICS.downloaded_storage_tries.lock().await;

        let remaining_storage_tries =
            total_storage_tries_to_download.saturating_sub(*downloaded_storage_tries);

        let current_storage_tries_download_progress = if *total_storage_tries_to_download == 0 {
            0.0
        } else {
            (*downloaded_storage_tries as f64 / *total_storage_tries_to_download as f64) * 100.0
        };

        // Storage tries state roots
        let total_storage_tries_state_roots_to_compute =
            METRICS.storage_tries_state_roots_to_compute.lock().await;

        let computed_storage_tries_state_roots = METRICS.storage_tries_state_roots_computed.get();

        let remaining_storage_tries_state_roots = total_storage_tries_state_roots_to_compute
            .saturating_sub(computed_storage_tries_state_roots);

        let current_storage_tries_state_roots_progress =
            if *total_storage_tries_state_roots_to_compute == 0 {
                0.0
            } else {
                (computed_storage_tries_state_roots as f64
                    / *total_storage_tries_state_roots_to_compute as f64)
                    * 100.0
            };

        let time_taken_to_compute_storage_tries_state_roots = {
            let end_time = METRICS
                .storage_tries_state_roots_end_time
                .lock()
                .await
                .unwrap_or(SystemTime::now());

            METRICS
                .storage_tries_state_roots_start_time
                .lock()
                .await
                .map(|start_time| {
                    end_time
                        .duration_since(start_time)
                        .expect("Failed to get storage tries state roots compute time")
                })
        };

        let time_taken_to_download_bytecodes = {
            let end_time = METRICS
                .bytecode_download_end_time
                .lock()
                .await
                .unwrap_or(SystemTime::now());

            METRICS
                .bytecode_download_start_time
                .lock()
                .await
                .map(|start_time| {
                    end_time
                        .duration_since(start_time)
                        .expect("Failed to get storage tries download time")
                })
        };

        let total_bytecodes_to_download = METRICS.bytecodes_to_download.lock().await;

        let downloaded_bytecodes = METRICS.downloaded_bytecodes.lock().await;

        let remaining_bytecodes = total_bytecodes_to_download.saturating_sub(*downloaded_bytecodes);

        let current_bytecodes_download_progress = if *total_bytecodes_to_download == 0 {
            0.0
        } else {
            (*downloaded_bytecodes as f64 / *total_bytecodes_to_download as f64) * 100.0
        };

        debug!(
            r#"
P2P:
====
elapsed: {elapsed}
{current_contacts} current contacts ({new_contacts_rate} contacts/m)
{discarded_nodes} discarded nodes
{discovered_nodes} total discovered nodes over time
{sent_pings} pings sent ({sent_pings_rate} new pings sent/m)
{peers} peers ({new_peers_rate} new peers/m)
{lost_peers} lost peers
{rlpx_connections} total peers made over time
{rlpx_connection_attempts} connection attempts ({new_rlpx_connection_attempts_rate} new connection attempts/m)
{rlpx_failed_connection_attempts} failed connection attempts
Clients diversity: {peers_by_client:#?}

Snap Sync:
==========
headers progress: {headers_download_progress} (total: {headers_to_download}, downloaded: {downloaded_headers}, remaining: {remaining_headers}, elapsed: {headers_download_time})
downloaded account tries: {downloaded_account_tries}, elapsed: {account_tries_download_time}
storage tries progress: {storage_tries_download_progress} (total: {storage_tries_to_download}, downloaded: {downloaded_storage_tries}, remaining: {remaining_storage_tries}, slots: {downloaded_storage_slots}, tasks: {storage_tries_tasks_queued}, elapsed: {storage_tries_download_time})
account tries state root: {account_tries_state_root}
storage tries state root progress: {storage_tries_state_roots_compute_progress} (total: {total_storage_tries_state_roots_to_compute}, computed: {computed_storage_tries_state_roots}, remaining: {remaining_storage_tries_state_roots}, elapsed: {storage_tries_state_roots_compute_time})
bytecodes progress: {bytecodes_download_progress} (total: {bytecodes_to_download}, downloaded: {downloaded_bytecodes}, remaining: {remaining_bytecodes}, elapsed: {bytecodes_download_time})"#,
            elapsed = format_duration(start.elapsed()),
            peers = METRICS.peers.lock().await,
            current_contacts = METRICS.contacts.lock().await,
            new_contacts_rate = METRICS.new_contacts_rate.get().floor(),
            discarded_nodes = METRICS.discarded_nodes.get(),
            discovered_nodes = METRICS.discovered_nodes.get(),
            sent_pings = METRICS.pings_sent.get(),
            sent_pings_rate = METRICS.pings_sent_rate.get().floor(),
            new_peers_rate = METRICS.new_connection_establishments_rate.get().floor(),
            lost_peers = rlpx_disconnections
                .values()
                .flat_map(|x| x.values())
                .sum::<u64>(),
            rlpx_connections = METRICS.connection_establishments.get(),
            rlpx_connection_attempts = METRICS.connection_attempts.get(),
            new_rlpx_connection_attempts_rate = METRICS.new_connection_attempts_rate.get().floor(),
            rlpx_failed_connection_attempts = rlpx_connection_failures.values().sum::<u64>(),
            peers_by_client = rlpx_connection_client_types,
            headers_download_progress = format!("{current_headers_download_progress:.2}%"),
            headers_to_download = total_headers_to_download,
            downloaded_headers = downloaded_headers,
            downloaded_account_tries = downloaded_account_tries,
            storage_tries_download_progress =
                format!("{current_storage_tries_download_progress:.2}%"),
            storage_tries_to_download = total_storage_tries_to_download,
            downloaded_storage_tries = downloaded_storage_tries,
            bytecodes_download_progress = format!("{current_bytecodes_download_progress:.2}%"),
            bytecodes_to_download = total_bytecodes_to_download,
            downloaded_bytecodes = downloaded_bytecodes,
            headers_download_time = maybe_headers_download_time
                .map(format_duration)
                .unwrap_or_else(|| "-".to_owned()),
            account_tries_download_time = time_taken_to_download_account_tries
                .map(format_duration)
                .unwrap_or_else(|| "-".to_owned()),
            storage_tries_download_time = time_taken_to_download_storage_tries
                .map(format_duration)
                .unwrap_or_else(|| "-".to_owned()),
            bytecodes_download_time = time_taken_to_download_bytecodes
                .map(format_duration)
                .unwrap_or_else(|| "-".to_owned()),
            account_tries_state_root = METRICS
                .account_tries_state_root
                .lock()
                .await
                .map(|state_root| format!("{state_root:#x}"),)
                .unwrap_or_else(|| "N/A".to_owned()),
            storage_tries_state_roots_compute_progress =
                format!("{current_storage_tries_state_roots_progress:.2}%"),
            total_storage_tries_state_roots_to_compute = total_storage_tries_state_roots_to_compute,
            computed_storage_tries_state_roots = computed_storage_tries_state_roots,
            remaining_storage_tries_state_roots = remaining_storage_tries_state_roots,
            storage_tries_state_roots_compute_time =
                time_taken_to_compute_storage_tries_state_roots
                    .map(format_duration)
                    .unwrap_or_else(|| "-".to_owned()),
            downloaded_storage_slots = *METRICS.downloaded_storage_slots.lock().await,
            storage_tries_tasks_queued = METRICS.storages_downloads_tasks_queued.lock().await,
        );

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Shows the amount of connected peers, active peers, and peers suitable for snap sync on a set interval
pub async fn periodically_show_peer_stats_after_sync(peers: Arc<Mutex<BTreeMap<H256, PeerData>>>) {
    const INTERVAL_DURATION: tokio::time::Duration = tokio::time::Duration::from_secs(60);
    let mut interval = tokio::time::interval(INTERVAL_DURATION);
    loop {
        // clone peers to keep the lock short
        let peers: Vec<PeerData> = peers.lock().await.values().cloned().collect();
        let active_peers = peers
            .iter()
            .filter(|peer| -> bool { peer.channels.as_ref().is_some() })
            .count();
        let snap_active_peers = peers
            .iter()
            .filter(|peer| -> bool {
                peer.channels.as_ref().is_some()
                    && SUPPORTED_SNAP_CAPABILITIES
                        .iter()
                        .any(|cap| peer.supported_capabilities.contains(cap))
            })
            .count();
        info!("Snap Peers: {snap_active_peers} / Total Peers: {active_peers}");
        interval.tick().await;
    }
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    let milliseconds = total_seconds / 1000;

    format!("{hours:02}h {minutes:02}m {seconds:02}s {milliseconds:02}ms")
}
