use crate::{
    peer::ingress::{Packet, PacketData},
    types::{Endpoint, Node, NodeId, NodeRecord},
};
use ethrex_core::{H256, H512};
use k256::{
    elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint},
    EncodedPoint, PublicKey,
};
use std::net::SocketAddr;

/// Computes public key from recipient id.
/// The node ID is the uncompressed public key of a node, with the first byte omitted (0x04).
pub fn id2pubkey(id: H512) -> Option<PublicKey> {
    let point = EncodedPoint::from_untagged_bytes(&id.0.into());
    PublicKey::from_encoded_point(&point).into_option()
}

/// Computes recipient id from public key.
pub fn pubkey2id(pk: &PublicKey) -> H512 {
    let encoded = pk.to_encoded_point(false);
    let bytes = encoded.as_bytes();
    debug_assert_eq!(bytes[0], 4);
    H512::from_slice(&bytes[1..])
}

pub fn new_expiration() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 20
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
        PacketData::ENRResponse { .. } | PacketData::Auth { .. } | PacketData::AuthAck { .. } => {
            false
        }
    }
}

pub fn new_ping(from: &Node, to: &SocketAddr) -> PacketData {
    PacketData::Ping {
        version: 4,
        from: from.endpoint.clone(),
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

pub fn hash_node_id(node_id: &NodeId) -> H512 {
    H512::from_slice(&node_id.serialize()[1..])
}
