use bootnode::BootNode;
use discv4::Discv4;
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

pub async fn start_network(
    tracker: TaskTracker,
    udp_addr: SocketAddr,
    tcp_addr: SocketAddr,
    bootnodes: Vec<BootNode>,
    signer: SigningKey,
    peer_table: Arc<Mutex<KademliaTable>>,
    storage: Store,
) {
    info!("Starting discovery service at {udp_addr}");
    info!("Listening for requests at {tcp_addr}");
    let (channel_broadcast_send_end, _) = tokio::sync::broadcast::channel::<(
        tokio::task::Id,
        Arc<RLPxMessage>,
    )>(MAX_MESSAGES_TO_BROADCAST);

    // TODO handle errors here
    let discovery = Discv4::try_new(
        Node {
            ip: udp_addr.ip(),
            udp_port: udp_addr.port(),
            tcp_port: tcp_addr.port(),
            node_id: H512::default(),
        },
        signer.clone(),
        storage.clone(),
        peer_table.clone(),
        channel_broadcast_send_end.clone(),
        tracker.clone(),
    )
    .await
    .unwrap();
    discovery.start(bootnodes).await.unwrap();

    tracker.spawn(serve_p2p_requests(
        tracker.clone(),
        tcp_addr,
        signer.clone(),
        storage.clone(),
        peer_table.clone(),
        channel_broadcast_send_end,
    ));
}

async fn serve_p2p_requests(
    tracker: TaskTracker,
    tcp_addr: SocketAddr,
    signer: SigningKey,
    storage: Store,
    table: Arc<Mutex<KademliaTable>>,
    connection_broadcast: RLPxConnBroadcastSender,
) {
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

        tracker.spawn(handle_peer_as_receiver(
            peer_addr,
            signer.clone(),
            stream,
            storage.clone(),
            table.clone(),
            connection_broadcast.clone(),
        ));
    }
}

fn listener(tcp_addr: SocketAddr) -> Result<TcpListener, io::Error> {
    let tcp_socket = TcpSocket::new_v4()?;
    tcp_socket.bind(tcp_addr)?;
    tcp_socket.listen(50)
}

async fn handle_peer_as_receiver(
    peer_addr: SocketAddr,
    signer: SigningKey,
    stream: TcpStream,
    storage: Store,
    table: Arc<Mutex<KademliaTable>>,
    connection_broadcast: RLPxConnBroadcastSender,
) {
    let mut conn = RLPxConnection::receiver(signer, stream, storage, connection_broadcast);
    conn.start_peer(peer_addr, table).await;
}

async fn handle_peer_as_initiator(
    signer: SigningKey,
    msg: &[u8],
    node: &Node,
    storage: Store,
    table: Arc<Mutex<KademliaTable>>,
    connection_broadcast: RLPxConnBroadcastSender,
) {
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
    match RLPxConnection::initiator(signer, msg, stream, storage, connection_broadcast) {
        Ok(mut conn) => {
            conn.start_peer(SocketAddr::new(node.ip, node.udp_port), table)
                .await
        }
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
    const INTERVAL_DURATION: tokio::time::Duration = tokio::time::Duration::from_secs(60);
    let mut interval = tokio::time::interval(INTERVAL_DURATION);
    loop {
        peer_table.lock().await.show_peer_stats();
        interval.tick().await;
    }
}
