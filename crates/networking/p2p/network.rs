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
        info!(
            r#"
elapsed: {}
{} current contacts ({} contacts/s)
{} discarded contacts
{} total contacts
{} peers ({} new peers/s)
{} connection attempts ({} new connection attempts/s)
{} failed connections
Known peers from mainnet: {}
RLPx connection failures: {:#?}"#,
            format_duration(start.elapsed()),
            METRICS.current_contacts.lock().await,
            METRICS.new_contacts_rate.get().floor(),
            METRICS.discarded_contacts.get(),
            METRICS.total_contacts.get(),
            METRICS.rlpx_conn_establishments.get(),
            METRICS.rlpx_conn_establishments_rate.get().floor(),
            METRICS.rlpx_conn_attempts.get(),
            METRICS.rlpx_conn_attempts_rate.get().floor(),
            METRICS.rlpx_conn_failures.get(),
            {
                let discovered = METRICS.discovered_mainnet_peers.lock().await.len();
                let we_failed_to_ping = METRICS.failed_to_ping_mainnet_peers.lock().await.len();
                let pinged = METRICS.pinged_mainnet_peers.lock().await.len();
                let answered_our_ping = METRICS.answered_our_ping_mainnet_peers.lock().await.len();
                let didnt_answer_our_ping = pinged - we_failed_to_ping - answered_our_ping;
                let connected = METRICS.connected_mainnet_peers.lock().await.len();
                let connection_attempts = METRICS
                    .connection_attempts_to_mainnet_peers
                    .lock()
                    .await
                    .len();
                let connection_failures = METRICS
                    .connection_failures_to_mainnet_peers
                    .lock()
                    .await
                    .len();
                serde_json::to_string_pretty(&serde_json::json!({
                    "discovered": discovered,
                    "we failed to ping": we_failed_to_ping,
                    "didn't answer our ping": didnt_answer_our_ping,
                    "pinged": pinged,
                    "answered our ping": answered_our_ping,
                    "connection attempts": connection_attempts,
                    "connection failures": connection_failures,
                    "connection failure reasons": *METRICS.connection_failures_to_mainnet_peers_reasons_counts.lock().await,
                    "connected": connected
                }))
                .expect("Failed to pritty print known mainnet peers counters")
            },
            METRICS.rlpx_conn_failures_reasons_counts.lock().await,
        );
        // info!(
        //     contacts = kademlia_clone.table.lock().await.len(),
        //     number_of_peers = number_of_peers,
        //     number_of_tried_peers = number_of_tried_peers,
        //     elapsed = format_duration(elapsed),
        //     new_contacts_rate = %format!("{} contacts/s", METRICS.new_contacts_rate.get().floor()),
        //     connection_attempts_rate = %format!("{} attempts/s", METRICS.attempted_rlpx_conn_rate.get().floor()),
        //     connection_establishments_rate = %format!("{} establishments/s", METRICS.established_rlpx_conn_rate.get().floor()),
        //     failed_connections = METRICS.failed_rlpx_conn.get(),
        // );
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
