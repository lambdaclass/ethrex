use std::{net::SocketAddr, sync::Arc};

use bootnode::BootNode;
use discv4::discv4::Discv4;
use ethrex_core::H512;
use ethrex_storage::Store;
use k256::{
    ecdsa::SigningKey,
    elliptic_curve::{sec1::ToEncodedPoint, PublicKey},
};
pub use kademlia::KademliaTable;
use rlpx::{connection::RLPxConnection, message::Message as RLPxMessage};
use tokio::{
    net::{TcpSocket, TcpStream, UdpSocket},
    sync::{broadcast, Mutex},
    try_join,
};
use tracing::{debug, error, info};
use types::Node;

pub mod bootnode;
pub(crate) mod discv4;
pub(crate) mod kademlia;
pub mod peer_channels;
pub mod rlpx;
pub(crate) mod snap;
pub mod sync;
pub mod types;

const MAX_DISC_PACKET_SIZE: usize = 1280;

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

    let udp_socket = UdpSocket::bind(udp_addr).await.unwrap();
    let local_node = Node {
        ip: udp_addr.ip(),
        node_id: node_id_from_signing_key(&signer),
        udp_port: udp_addr.port(),
        tcp_port: tcp_addr.port(),
    };

    let discv4 = Discv4::new(
        local_node,
        signer.clone(),
        storage.clone(),
        peer_table.clone(),
        channel_broadcast_send_end.clone(),
        Arc::new(udp_socket),
    );
    let discv4 = Arc::new(discv4);
    let discovery_handle = tokio::spawn(discv4.start_discovery_service(bootnodes));

    let server_handle = tokio::spawn(serve_requests(
        tcp_addr,
        signer.clone(),
        storage.clone(),
        peer_table.clone(),
        channel_broadcast_send_end,
    ));

    let _ = try_join!(discovery_handle, server_handle).unwrap();
}

async fn serve_requests(
    tcp_addr: SocketAddr,
    signer: SigningKey,
    storage: Store,
    table: Arc<Mutex<KademliaTable>>,
    connection_broadcast: broadcast::Sender<(tokio::task::Id, Arc<RLPxMessage>)>,
) {
    let tcp_socket = TcpSocket::new_v4().unwrap();
    tcp_socket.bind(tcp_addr).unwrap();
    let listener = tcp_socket.listen(50).unwrap();
    loop {
        let (stream, _peer_addr) = listener.accept().await.unwrap();

        tokio::spawn(handle_peer_as_receiver(
            signer.clone(),
            stream,
            storage.clone(),
            table.clone(),
            connection_broadcast.clone(),
        ));
    }
}

async fn handle_peer_as_receiver(
    signer: SigningKey,
    stream: TcpStream,
    storage: Store,
    table: Arc<Mutex<KademliaTable>>,
    connection_broadcast: broadcast::Sender<(tokio::task::Id, Arc<RLPxMessage>)>,
) {
    let mut conn = RLPxConnection::receiver(signer, stream, storage, connection_broadcast);
    conn.start_peer(table).await;
}

async fn handle_peer_as_initiator(
    signer: SigningKey,
    msg: &[u8],
    node: &Node,
    storage: Store,
    table: Arc<Mutex<KademliaTable>>,
    connection_broadcast: broadcast::Sender<(tokio::task::Id, Arc<RLPxMessage>)>,
) {
    debug!("Trying RLPx connection with {node:?}");
    let stream = TcpSocket::new_v4()
        .unwrap()
        .connect(SocketAddr::new(node.ip, node.tcp_port))
        .await
        .unwrap();
    match RLPxConnection::initiator(signer, msg, stream, storage, connection_broadcast) {
        Ok(mut conn) => conn.start_peer(table).await,
        Err(e) => {
            error!("Error: {e}, Could not start connection with {node:?}");
        }
    }
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
