use crate::kademlia::{self, KademliaTable};
use crate::rlpx::p2p::Capability;
use crate::rlpx::{
    connection::RLPxConnBroadcastSender, handshake, message::Message as RLPxMessage,
};
use crate::types::Node;
use crate::{
    discv4::{
        helpers::current_unix_time,
        server::{DiscoveryError, Discv4Server},
    },
    rlpx::utils::log_peer_error,
};
use ethrex_blockchain::Blockchain;
use ethrex_common::H512;
use k256::{
    ecdsa::SigningKey,
    elliptic_curve::{sec1::ToEncodedPoint, PublicKey},
};
use std::{io, net::SocketAddr, sync::Arc};
use tokio::{
    net::{TcpListener, TcpSocket, TcpStream},
    sync::Mutex,
};
use tokio_util::task::TaskTracker;
use tracing::{debug, error, info};

// Totally arbitrary limit on how
// many messages the connections can queue,
// if we miss messages to broadcast, maybe
// we should bump this limit.
pub const MAX_MESSAGES_TO_BROADCAST: usize = 1000;

pub fn peer_table(signer: SigningKey) -> Arc<Mutex<KademliaTable>> {
    let local_node_id = node_id_from_signing_key(&signer);
    Arc::new(Mutex::new(KademliaTable::new(local_node_id)))
}

#[derive(Debug)]
pub enum NetworkError {
    DiscoveryStart(DiscoveryError),
}

#[derive(Clone, Debug)]
pub struct P2PContext {
    pub tracker: TaskTracker,
    pub signer: SigningKey,
    pub table: Arc<Mutex<KademliaTable>>,
    pub blockchain: Blockchain,
    pub(crate) broadcast: RLPxConnBroadcastSender,
    pub local_node: Node,
    pub enr_seq: u64,
}

pub async fn start_network(
    local_node: Node,
    tracker: TaskTracker,
    bootnodes: Vec<Node>,
    signer: SigningKey,
    peer_table: Arc<Mutex<KademliaTable>>,
    blockchain: Blockchain,
) -> Result<(), NetworkError> {
    let (channel_broadcast_send_end, _) = tokio::sync::broadcast::channel::<(
        tokio::task::Id,
        Arc<RLPxMessage>,
    )>(MAX_MESSAGES_TO_BROADCAST);

    let context = P2PContext {
        local_node,
        // Note we are passing the current timestamp as the sequence number
        // This is because we are not storing our local_node updates in the db
        // see #1756
        enr_seq: current_unix_time(),
        tracker,
        signer,
        table: peer_table,
        blockchain,
        broadcast: channel_broadcast_send_end,
    };
    let discovery = Discv4Server::try_new(context.clone())
        .await
        .map_err(NetworkError::DiscoveryStart)?;

    info!(
        "Starting discovery service at {}",
        context.local_node.udp_addr()
    );
    discovery
        .start(bootnodes)
        .await
        .map_err(NetworkError::DiscoveryStart)?;

    info!(
        "Listening for requests at {}",
        context.local_node.tcp_addr()
    );
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

        context
            .tracker
            .spawn(handle_peer_as_receiver(context.clone(), peer_addr, stream));
    }
}

fn listener(tcp_addr: SocketAddr) -> Result<TcpListener, io::Error> {
    let tcp_socket = TcpSocket::new_v4()?;
    tcp_socket.bind(tcp_addr)?;
    tcp_socket.listen(50)
}

async fn handle_peer_as_receiver(context: P2PContext, peer_addr: SocketAddr, stream: TcpStream) {
    let table = context.table.clone();
    match handshake::as_receiver(context, peer_addr, stream).await {
        Ok(mut conn) => conn.start(table).await,
        Err(e) => {
            debug!("Error creating tcp connection with peer at {peer_addr}: {e}")
        }
    }
}

pub async fn handle_peer_as_initiator(context: P2PContext, node: Node) {
    let addr = SocketAddr::new(node.ip, node.tcp_port);
    let stream = match tcp_stream(addr).await {
        Ok(result) => result,
        Err(e) => {
            log_peer_error(&node, &format!("Error creating tcp connection {e}"));
            context.table.lock().await.replace_peer(node.node_id);
            return;
        }
    };
    let table = context.table.clone();
    match handshake::as_initiator(context, node, stream).await {
        Ok(mut conn) => conn.start(table).await,
        Err(e) => {
            log_peer_error(&node, &format!("Error creating tcp connection {e}"));
            table.lock().await.replace_peer(node.node_id);
        }
    };
}

async fn tcp_stream(addr: SocketAddr) -> Result<TcpStream, io::Error> {
    TcpSocket::new_v4()?.connect(addr).await
}

pub fn node_id_from_signing_key(signer: &SigningKey) -> H512 {
    let public_key = PublicKey::from(signer.verifying_key());
    let encoded = public_key.to_encoded_point(false);
    H512::from_slice(&encoded.as_bytes()[1..])
}

/// Shows the amount of connected peers, active peers, and peers suitable for snap sync on a set interval
pub async fn periodically_show_peer_stats(peer_table: Arc<Mutex<KademliaTable>>) {
    const INTERVAL_DURATION: tokio::time::Duration = tokio::time::Duration::from_secs(30);
    let mut interval = tokio::time::interval(INTERVAL_DURATION);
    loop {
        // clone peers to keep the lock short
        let peers: Vec<kademlia::PeerData> =
            peer_table.lock().await.iter_peers().cloned().collect();
        let total_peers = peers.len();
        let active_peers = peers
            .iter()
            .filter(|peer| -> bool { peer.channels.as_ref().is_some() })
            .count();
        let snap_active_peers = peers
            .iter()
            .filter(|peer| -> bool {
                peer.channels.as_ref().is_some()
                    && peer.supported_capabilities.contains(&Capability::Snap)
            })
            .count();
        info!("Snap Peers: {snap_active_peers} / Active Peers {active_peers} / Total Peers: {total_peers}");
        interval.tick().await;
    }
}
