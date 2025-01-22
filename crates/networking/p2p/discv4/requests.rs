use std::{
    net::SocketAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ethrex_core::{H256, H512};
use k256::ecdsa::SigningKey;
use tokio::net::UdpSocket;

use crate::{
    kademlia::MAX_NODES_PER_BUCKET,
    types::{Endpoint, Node},
};

use super::messages::{FindNodeMessage, Message, PingMessage, PongMessage};

// Sends a ping to the addr
/// # Returns
/// an optional hash corresponding to the message header hash to account if the send was successful
pub async fn ping(
    socket: &UdpSocket,
    local_node: Node,
    to_node: Node,
    signer: &SigningKey,
) -> Option<H256> {
    let mut buf = Vec::new();

    let expiration: u64 = (SystemTime::now() + Duration::from_secs(20))
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let from = Endpoint {
        ip: local_node.ip,
        udp_port: local_node.udp_port,
        tcp_port: local_node.tcp_port,
    };
    let to = Endpoint {
        ip: to_node.ip,
        udp_port: to_node.udp_port,
        tcp_port: to_node.tcp_port,
    };

    let ping = Message::Ping(PingMessage::new(from, to, expiration));
    ping.encode_with_header(&mut buf, signer);
    let res = socket
        .send_to(&buf, SocketAddr::new(to_node.ip, to_node.udp_port))
        .await;

    if res.is_err() {
        return None;
    }
    let bytes_sent = res.unwrap();

    if bytes_sent == buf.len() {
        return Some(H256::from_slice(&buf[0..32]));
    }

    None
}

pub async fn pong(
    socket: &UdpSocket,
    to_addr: SocketAddr,
    node: Node,
    ping_hash: H256,
    signer: &SigningKey,
) {
    let mut buf = Vec::new();

    let expiration: u64 = (SystemTime::now() + Duration::from_secs(20))
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let to = Endpoint {
        ip: node.ip,
        udp_port: node.udp_port,
        tcp_port: node.tcp_port,
    };
    let pong = Message::Pong(PongMessage::new(to, ping_hash, expiration));

    pong.encode_with_header(&mut buf, signer);
    let _ = socket.send_to(&buf, to_addr).await;
}

pub async fn find_node_and_wait_for_response(
    socket: &UdpSocket,
    to_addr: SocketAddr,
    signer: &SigningKey,
    target_node_id: H512,
    request_receiver: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<Node>>,
) -> Vec<Node> {
    let expiration: u64 = (SystemTime::now() + Duration::from_secs(20))
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let msg = Message::FindNode(FindNodeMessage::new(target_node_id, expiration));

    let mut buf = Vec::new();
    msg.encode_with_header(&mut buf, signer);
    let res = socket.send_to(&buf, to_addr).await;

    let mut nodes = vec![];

    if res.is_err() {
        return nodes;
    }

    loop {
        // wait as much as 5 seconds for the response
        match tokio::time::timeout(Duration::from_secs(5), request_receiver.recv()).await {
            Ok(Some(mut found_nodes)) => {
                nodes.append(&mut found_nodes);
                if nodes.len() == MAX_NODES_PER_BUCKET {
                    return nodes;
                };
            }
            Ok(None) => {
                return nodes;
            }
            Err(_) => {
                // timeout expired
                return nodes;
            }
        }
    }
}
