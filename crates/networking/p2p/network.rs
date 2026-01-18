#[cfg(feature = "l2")]
use crate::rlpx::l2::l2_connection::P2PBasedContext;
#[cfg(not(feature = "l2"))]
#[derive(Clone, Debug)]
pub struct P2PBasedContext;
use crate::{
    discovery_server::{DiscoveryServer, DiscoveryServerError},
    peer_table::{PeerData, PeerTable},
    rlpx::{
        connection::server::{PeerConnBroadcastSender, PeerConnection},
        message::Message,
        p2p::SUPPORTED_SNAP_CAPABILITIES,
    },
    tx_broadcaster::{TxBroadcaster, TxBroadcasterError},
    types::Node,
};
use ethrex_blockchain::Blockchain;
use ethrex_storage::Store;
use secp256k1::SecretKey;
use spawned_concurrency::tasks::GenServerHandle;
use std::{
    io,
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};
use tokio::net::{TcpListener, TcpSocket, UdpSocket};
use tokio_util::task::TaskTracker;
use tracing::{error, info};

pub const MAX_MESSAGES_TO_BROADCAST: usize = 100000;

#[derive(Clone, Debug)]
pub struct P2PContext {
    pub tracker: TaskTracker,
    pub signer: SecretKey,
    pub table: PeerTable,
    pub storage: Store,
    pub blockchain: Arc<Blockchain>,
    pub(crate) broadcast: PeerConnBroadcastSender,
    pub local_node: Node,
    pub client_version: String,
    #[cfg(feature = "l2")]
    pub based_context: Option<P2PBasedContext>,
    pub tx_broadcaster: GenServerHandle<TxBroadcaster>,
    pub initial_lookup_interval: f64,
}

impl P2PContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        local_node: Node,
        tracker: TaskTracker,
        signer: SecretKey,
        peer_table: PeerTable,
        storage: Store,
        blockchain: Arc<Blockchain>,
        client_version: String,
        based_context: Option<P2PBasedContext>,
        tx_broadcasting_time_interval: u64,
        lookup_interval: f64,
    ) -> Result<Self, NetworkError> {
        let (channel_broadcast_send_end, _) = tokio::sync::broadcast::channel::<(
            tokio::task::Id,
            Arc<Message>,
        )>(MAX_MESSAGES_TO_BROADCAST);

        let tx_broadcaster = TxBroadcaster::spawn(
            peer_table.clone(),
            blockchain.clone(),
            tx_broadcasting_time_interval,
        )
        .inspect_err(|e| {
            error!("Failed to start Tx Broadcaster: {e}");
        })?;

        #[cfg(not(feature = "l2"))]
        let _ = &based_context;

        Ok(P2PContext {
            local_node,
            tracker,
            signer,
            table: peer_table,
            storage,
            blockchain,
            broadcast: channel_broadcast_send_end,
            client_version,
            #[cfg(feature = "l2")]
            based_context,
            tx_broadcaster,
            initial_lookup_interval: lookup_interval,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("Failed to start discovery server: {0}")]
    DiscoveryServerError(#[from] DiscoveryServerError),
    #[error("Failed to start Tx Broadcaster: {0}")]
    TxBroadcasterError(#[from] TxBroadcasterError),
}

pub async fn start_network(context: P2PContext, bootnodes: Vec<Node>) -> Result<(), NetworkError> {
    let udp_socket = UdpSocket::bind(context.local_node.udp_addr())
        .await
        .expect("Failed to bind udp socket");

    DiscoveryServer::spawn(
        context.storage.clone(),
        context.local_node.clone(),
        context.signer,
        udp_socket,
        context.table.clone(),
        bootnodes,
        context.initial_lookup_interval,
    )
    .await
    .inspect_err(|e| {
        error!("Failed to start discovery server: {e}");
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

        let _ = PeerConnection::spawn_as_receiver(context.clone(), peer_addr, stream);
    }
}

fn listener(tcp_addr: SocketAddr) -> Result<TcpListener, io::Error> {
    let tcp_socket = match tcp_addr {
        SocketAddr::V4(_) => TcpSocket::new_v4(),
        SocketAddr::V6(_) => TcpSocket::new_v6(),
    }?;
    tcp_socket.set_reuseport(true).ok();
    tcp_socket.set_reuseaddr(true).ok();
    tcp_socket.bind(tcp_addr)?;

    tcp_socket.listen(50)
}

pub async fn periodically_show_peer_stats(blockchain: Arc<Blockchain>, mut peer_table: PeerTable) {
    // Wait for sync to complete before showing peer stats
    loop {
        if blockchain.is_synced() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    periodically_show_peer_stats_after_sync(&mut peer_table).await;
}

/// Shows the amount of connected peers, active peers, and peers suitable for snap sync on a set interval
pub async fn periodically_show_peer_stats_after_sync(peer_table: &mut PeerTable) {
    const INTERVAL_DURATION: tokio::time::Duration = tokio::time::Duration::from_secs(60);
    let mut interval = tokio::time::interval(INTERVAL_DURATION);
    loop {
        // clone peers to keep the lock short
        let peers: Vec<PeerData> = peer_table.get_peers_data().await.unwrap_or(Vec::new());
        let active_peers = peers
            .iter()
            .filter(|peer| -> bool { peer.connection.as_ref().is_some() })
            .count();
        let snap_active_peers = peers
            .iter()
            .filter(|peer| -> bool {
                peer.connection.as_ref().is_some()
                    && SUPPORTED_SNAP_CAPABILITIES
                        .iter()
                        .any(|cap| peer.supported_capabilities.contains(cap))
            })
            .count();
        info!("Snap Peers: {snap_active_peers} / Total Peers: {active_peers}");
        interval.tick().await;
    }
}
