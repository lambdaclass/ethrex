use discv4::{
    helpers::current_unix_time,
    server::{DiscoveryError, Discv4BackendMsg, Discv4Server},
};
use ethrex_core::H512;
use ethrex_storage::Store;
use k256::{
    ecdsa::SigningKey,
    elliptic_curve::{sec1::ToEncodedPoint, PublicKey},
};
pub use kademlia::KademliaTable;
use kademlia::PeerData;
use rlpx::{
    connection::RLPxConnBroadcastSender, handshake, message::Message as RLPxMessage,
    p2p::Capability, utils::log_peer_error,
};
use std::{fmt, io, net::SocketAddr, str::FromStr, sync::Arc};
use tokio::{
    net::{TcpListener, TcpSocket, TcpStream},
    sync::Mutex,
};
use tokio_util::task::TaskTracker;
use tracing::{error, info};
use types::Node;

pub mod discv4;
pub mod kademlia;
pub mod peer_handler;
pub mod rlpx;
pub(crate) mod snap;
pub mod sync;
pub mod types;

// Totally arbitrary limit on how
// many messages the connections can queue,
// if we miss messages to broadcast, maybe
// we should bump this limit.
const MAX_MESSAGES_TO_BROADCAST: usize = 1000;

pub fn peer_table(signer: SigningKey) -> Arc<Mutex<KademliaTable>> {
    let local_node_id = node_id_from_signing_key(&signer);
    Arc::new(Mutex::new(KademliaTable::new(local_node_id)))
}

#[derive(Debug)]
pub enum NetworkError {
    DiscoveryStart(DiscoveryError),
}

const NODE_ID_LOG_TO_SHOW: &str = "709764f16bc0867bc3f51c95e6c0ce9cb53fe679c7630acbc4aee3d7d7243d8937e84aba5253d8da835b1e603c6faa9db7676c6739749d558e9611b3ff791848";
pub fn node_id_log_to_show() -> H512 {
    H512::from_str(NODE_ID_LOG_TO_SHOW).unwrap()
}

#[derive(Clone, Debug)]
struct P2PContext {
    tracker: TaskTracker,
    signer: SigningKey,
    table: Arc<Mutex<KademliaTable>>,
    storage: Store,
    broadcast: RLPxConnBroadcastSender,
    local_node: Node,
    enr_seq: u64,
}

pub async fn start_network(
    local_node: Node,
    tracker: TaskTracker,
    bootnodes: Vec<Node>,
    signer: SigningKey,
    peer_table: Arc<Mutex<KademliaTable>>,
    storage: Store,
    discv4_backend_receiver: tokio::sync::mpsc::UnboundedReceiver<Discv4BackendMsg>,
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
        storage,
        broadcast: channel_broadcast_send_end.clone(),
    };
    let discovery = Arc::new(
        Discv4Server::try_new(context.clone())
            .await
            .map_err(NetworkError::DiscoveryStart)?,
    );

    info!(
        "Starting discovery service at {}",
        context.local_node.udp_addr()
    );
    discovery
        .start(bootnodes, discv4_backend_receiver)
        .await
        .map_err(NetworkError::DiscoveryStart)?;

    info!(
        "Listening for requests at {}",
        context.local_node.tcp_addr()
    );
    context.tracker.spawn(serve_p2p_requests(context.clone()));

    Ok(())
}

async fn serve_p2p_requests(context: P2PContext) {
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
            // log_peer_error(&node, &format!("Error creating tcp connection {e}"));
        }
    }
}

async fn handle_peer_as_initiator(context: P2PContext, node: Node) {
    let addr = SocketAddr::new(node.ip, node.tcp_port);
    let stream = match tcp_stream(addr).await {
        Ok(result) => result,
        Err(e) => {
            context.table.lock().await.replace_peer(node.node_id);
            log_peer_error(&node, &format!("Error creating tcp connection {e}"));
            return;
        }
    };
    let table = context.table.clone();
    match handshake::as_initiator(context, node, stream).await {
        Ok(mut conn) => conn.start(table).await,
        Err(e) => {
            // TODO We should remove the peer from the table if connection failed
            // but currently it will make the tests fail
            table.lock().await.replace_peer(node.node_id);
            log_peer_error(&node, &format!("Error creating tcp connection {e}"));
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

struct DebugPeer {
    enode_url: String,
    node: Node,
    capabilities: Vec<Capability>,
}

impl std::fmt::Debug for DebugPeer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\n ")?;
        writeln!(f, "'\t- node_id: {}", self.node.node_id)?;
        writeln!(f, "'\t- enode_url: {}", self.enode_url)?;
        writeln!(f, "'\t- capabilites: {:?}", self.capabilities)?;

        Ok(())
    }
}

/// Shows the amount of connected peers, active peers, and peers suitable for snap sync on a set interval
pub async fn periodically_show_peer_stats(peer_table: Arc<Mutex<KademliaTable>>) {
    const INTERVAL_DURATION: tokio::time::Duration = tokio::time::Duration::from_secs(10);
    let mut interval = tokio::time::interval(INTERVAL_DURATION);
    loop {
        let active_filter = |peer: &PeerData| -> bool { peer.channels.as_ref().is_some() };
        let peers: Vec<PeerData> = peer_table
            .lock()
            .await
            .filter_peers(&active_filter)
            .cloned()
            .collect();

        let peers: Vec<DebugPeer> = peers
            .iter()
            .map(|p| DebugPeer {
                node: p.node,
                enode_url: p.node.enode_url(),
                capabilities: p.supported_capabilities.clone(),
            })
            .collect();

        info!(
            "Peer stats:\n\t- discovered peers: {}\n\t- {:?}",
            peer_table.lock().await.iter_peers().count(),
            peers
        );

        interval.tick().await;
    }
}
