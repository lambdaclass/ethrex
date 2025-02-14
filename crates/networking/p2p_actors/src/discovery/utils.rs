use crate::{
    discovery::packet::{Packet, PacketData},
    types::{Endpoint, Node, NodeId, NodeRecord, NodeState, PeerData},
};
use ethrex_core::{H256, H512, U256};
use keccak_hash::keccak;
use std::{
    cmp::Reverse,
    collections::BTreeMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::sync::Mutex;

pub const PROOF_EXPIRATION_IN_SECONDS: u64 = 12 * 60 * 60;

pub fn new_expiration() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 20
}

pub fn elapsed_time_since(unix_timestamp: u64) -> u64 {
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(unix_timestamp);
    SystemTime::now().duration_since(time).unwrap().as_secs()
}

pub fn current_unix_time() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn is_last_ping_expired(last_ping: u64) -> bool {
    let expiration = last_ping + PROOF_EXPIRATION_IN_SECONDS;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        > expiration
}

pub fn is_expired(packet: &Packet) -> bool {
    match packet.data {
        PacketData::Ping { expiration, .. }
        | PacketData::Pong { expiration, .. }
        | PacketData::FindNode { expiration, .. }
        | PacketData::Neighbors { expiration, .. }
        | PacketData::ENRRequest { expiration, .. } => {
            // this cast to a signed integer is needed as the rlp decoder doesn't take into account the sign
            // otherwise a potential negative expiration would pass since it would take 2^64.
            (expiration as i64)
                < std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64
        }
        PacketData::ENRResponse { .. } => false,
    }
}

pub fn new_ping(from: Endpoint, to: &SocketAddr) -> PacketData {
    PacketData::Ping {
        version: 4,
        from,
        to: Endpoint::new(to.ip(), to.port(), 0),
        expiration: new_expiration(),
        enr_seq: None,
    }
}

pub fn new_pong(to: Endpoint, ping_hash: H256) -> PacketData {
    PacketData::Pong {
        to,
        ping_hash,
        expiration: new_expiration(),
        enr_seq: None,
    }
}

pub fn new_find_node(target: NodeId) -> PacketData {
    PacketData::FindNode {
        target,
        expiration: new_expiration(),
    }
}

pub fn new_neighbors(nodes: Vec<Node>) -> PacketData {
    PacketData::Neighbors {
        nodes,
        expiration: new_expiration(),
    }
}

pub fn new_enr_request() -> PacketData {
    PacketData::ENRRequest {
        expiration: new_expiration(),
    }
}

pub fn new_enr_response(request_hash: H256, node_record: NodeRecord) -> PacketData {
    PacketData::ENRResponse {
        request_hash,
        node_record,
    }
}

pub fn serialize_node_id(node_id: &NodeId) -> H512 {
    H512::from_slice(&node_id.serialize()[1..])
}

pub async fn neighbors(of: NodeId, from: Arc<Mutex<BTreeMap<SocketAddr, PeerData>>>) -> Vec<Node> {
    let table_lock = from.lock().await;
    let mut distances_to_target = Vec::new();
    for known_peer in table_lock.values() {
        let n1 = keccak(serialize_node_id(&of));
        let n2 = keccak(serialize_node_id(&known_peer.id));
        let distance = U256::from_big_endian((n1 ^ n2).as_bytes());
        distances_to_target.push((distance, known_peer));
    }

    distances_to_target.sort_by_key(|(distance, _)| Reverse(*distance));

    distances_to_target
        .iter()
        .filter(|(_, peer)| matches!(peer.state, NodeState::Proven { .. }))
        .take(16)
        .cloned()
        .map(|(_, peer)| Node {
            endpoint: peer.endpoint.clone(),
            id: peer.id,
        })
        .collect()
}
