use crate::{
    discv4::{
        server::{DiscoveryServer, DiscoveryServerError},
        side_car::{DiscoverySideCar, DiscoverySideCarError},
    },
    kademlia::Kademlia,
    metrics::METRICS,
    rlpx::{
        connection::server::{RLPxConnBroadcastSender, RLPxConnection},
        initiator::{RLPxInitiator, RLPxInitiatorError},
        message::Message,
    },
    types::{Node, NodeRecord},
};
use ethrex_blockchain::Blockchain;
use ethrex_common::types::ForkId;
use ethrex_storage::Store;
use k256::ecdsa::SigningKey;
use std::{io, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpSocket, UdpSocket},
    sync::Mutex,
};
use tokio_util::task::TaskTracker;
use tracing::{error, info};

pub const MAX_MESSAGES_TO_BROADCAST: usize = 100000;

#[derive(Clone, Debug)]
pub struct P2PContext {
    pub tracker: TaskTracker,
    pub signer: SigningKey,
    pub table: Kademlia,
    pub storage: Store,
    pub blockchain: Arc<Blockchain>,
    pub(crate) broadcast: RLPxConnBroadcastSender,
    pub local_node: Node,
    pub local_node_record: Arc<Mutex<NodeRecord>>,
    pub client_version: String,
}

impl P2PContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        local_node: Node,
        local_node_record: Arc<Mutex<NodeRecord>>,
        tracker: TaskTracker,
        signer: SigningKey,
        peer_table: Kademlia,
        storage: Store,
        blockchain: Arc<Blockchain>,
        client_version: String,
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
}

pub fn peer_table() -> Kademlia {
    Kademlia::new()
}

pub async fn start_network(
    context: P2PContext,
    bootnodes: Vec<Node>,
    fork_id: &ForkId,
) -> Result<(), NetworkError> {
    let udp_socket = Arc::new(
        UdpSocket::bind(context.local_node.udp_addr())
            .await
            .expect("Failed to bind udp socket"),
    );

    DiscoveryServer::spawn(
        context.local_node.clone(),
        context.signer.clone(),
        fork_id,
        udp_socket.clone(),
        context.table.clone(),
        bootnodes,
    )
    .await
    .inspect_err(|e| {
        error!("Failed to start discovery server: {e}");
    })?;

    DiscoverySideCar::spawn(
        context.local_node.clone(),
        context.signer.clone(),
        fork_id,
        udp_socket,
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

        let _ = RLPxConnection::spawn_as_receiver(context.clone(), peer_addr, stream).await;
    }
}

fn listener(tcp_addr: SocketAddr) -> Result<TcpListener, io::Error> {
    let tcp_socket = TcpSocket::new_v4()?;
    tcp_socket.bind(tcp_addr)?;
    tcp_socket.listen(50)
}

pub async fn periodically_show_peer_stats() {
    let start = std::time::Instant::now();
    loop {
        let rlpx_connection_failures = METRICS.connection_attempt_failures.lock().await;

        let rlpx_disconnections = METRICS.disconnections.lock().await;

        let total_downloaders = METRICS.total_downloaders.lock().await;
        let free_downloaders = METRICS.free_downloaders.lock().await;
        let busy_downloaders = total_downloaders.saturating_sub(*free_downloaders);

        let total_headers_to_download = METRICS.headers_to_download.lock().await;
        let downloaded_headers = METRICS.downloaded_headers.lock().await;
        let remaining_headers = total_headers_to_download.saturating_sub(*downloaded_headers);

        info!(
            r#"
elapsed: {elapsed}
{current_contacts} current contacts ({new_contacts_rate} contacts/m)
{discarded_nodes} discarded nodes
{discovered_nodes} total discovered nodes over time
{peers} peers ({new_peers_rate} new peers/m)
{lost_peers} lost peers
{rlpx_connections} total peers made over time
{rlpx_connection_attempts} connection attempts ({new_rlpx_connection_attempts_rate} new connection attempts/m)
{rlpx_failed_connection_attempts} failed connection attempts
Sync head hash: {sync_head_hash:#x}
Sync head block: {sync_head_block}
Headers to download: {headers_to_download}
Downloaded headers: {downloaded_headers}
Remaining headers: {remaining_headers}
Total downloaders: {total_downloaders}
Busy downloaders: {busy_downloaders}
Free downloaders: {free_downloaders}
Tasks queued: {tasks_queued}
RLPx disconnections: {rlpx_disconnections:#?}
RLPx connection failures: {rlpx_connection_failures_grouped_and_counted_by_reason:#?}"#,
            elapsed = format_duration(start.elapsed()),
            current_contacts = METRICS.contacts.lock().await,
            new_contacts_rate = METRICS.new_contacts_rate.get().floor(),
            discarded_nodes = METRICS.discarded_nodes.get(),
            discovered_nodes = METRICS.discovered_nodes.get(),
            peers = METRICS.peers.lock().await,
            new_peers_rate = METRICS.new_connection_establishments_rate.get().floor(),
            lost_peers = rlpx_disconnections.values().sum::<u64>(),
            rlpx_connections = METRICS.connection_establishments.get(),
            rlpx_connection_attempts = METRICS.connection_attempts.get(),
            new_rlpx_connection_attempts_rate = METRICS.new_connection_attempts_rate.get().floor(),
            rlpx_failed_connection_attempts = rlpx_connection_failures.values().sum::<u64>(),
            sync_head_hash = *METRICS.sync_head_hash.lock().await,
            sync_head_block = METRICS.sync_head_block.lock().await,
            headers_to_download = total_headers_to_download,
            downloaded_headers = downloaded_headers,
            remaining_headers = remaining_headers,
            total_downloaders = total_downloaders,
            busy_downloaders = busy_downloaders,
            free_downloaders = free_downloaders,
            tasks_queued = METRICS.tasks_queued.lock().await,
            rlpx_disconnections = rlpx_disconnections,
            rlpx_connection_failures_grouped_and_counted_by_reason = rlpx_connection_failures,
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}
