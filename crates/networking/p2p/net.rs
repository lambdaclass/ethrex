use bootnode::BootNode;
use discv4::{
    helpers::current_unix_time,
    server::{DiscoveryError, Discv4Server},
};
use ethrex_core::H512;
use ethrex_storage::Store;
use k256::{
    ecdsa::SigningKey,
    elliptic_curve::{sec1::ToEncodedPoint, PublicKey},
};
pub use kademlia::KademliaTable;
use rlpx::{
    connection::{RLPxConnBroadcastSender, RLPxConnection},
    message::Message as RLPxMessage,
};
use std::{io, net::SocketAddr, sync::Arc};
use tokio::{
    net::{TcpListener, TcpSocket, TcpStream},
    sync::Mutex,
};
use tokio_util::task::TaskTracker;
use tracing::{error, info};
use types::Node;

pub mod bootnode;
pub(crate) mod discv4;
pub(crate) mod kademlia;
pub mod peer_channels;
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
    bootnodes: Vec<BootNode>,
    signer: SigningKey,
    peer_table: Arc<Mutex<KademliaTable>>,
    storage: Store,
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
    let mut conn =
        RLPxConnection::receiver(context.signer, stream, context.storage, context.broadcast);
    conn.start_peer(peer_addr, context.table).await;
}

async fn handle_peer_as_initiator(context: P2PContext, node: Node) {
    let addr = SocketAddr::new(node.ip, node.tcp_port);
    let stream = match tcp_stream(addr).await {
        Ok(result) => result,
        Err(e) => {
            // TODO We should remove the peer from the table if connection failed
            // but currently it will make the tests fail
            // table.lock().await.replace_peer(node.node_id);
            error!("Error establishing tcp connection with peer at {addr}: {e}");
            return;
        }
    };
    match RLPxConnection::initiator(
        context.signer,
        node.node_id,
        stream,
        context.storage,
        context.broadcast,
    ) {
        Ok(mut conn) => conn.start_peer(node.udp_addr(), context.table).await,
        Err(e) => {
            // TODO We should remove the peer from the table if connection failed
            // but currently it will make the tests fail
            // table.lock().await.replace_peer(node.node_id);
            error!("Error creating tcp connection with peer at {addr}: {e}")
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
    const INTERVAL_DURATION: tokio::time::Duration = tokio::time::Duration::from_secs(120);
    let mut interval = tokio::time::interval(INTERVAL_DURATION);
    loop {
        peer_table.lock().await.show_peer_stats();
        interval.tick().await;
    }
}
