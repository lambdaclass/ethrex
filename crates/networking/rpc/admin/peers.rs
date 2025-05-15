use crate::{rpc::RpcApiContext, utils::RpcErr};
use core::net::SocketAddr;
use ethrex_common::H256;
use ethrex_p2p::{kademlia::PeerData, rlpx::p2p::Capability};
use serde::Serialize;
use serde_json::Value;

/// Serializable peer data returned by the node's rpc
#[derive(Serialize)]
pub struct RpcPeer {
    caps: Vec<Capability>,
    enode: String,
    id: H256,
    network: PeerNetwork,
    protocols: Protocols,
}

/// Serializable peer network data returned by the node's rpc
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PeerNetwork {
    // We can add more data about the connection here, such the local address, wether the peer is trusted, etc
    inbound: bool,
    remote_address: SocketAddr,
}

/// Serializable peer protocols data returned by the node's rpc
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Protocols {
    #[serde(skip_serializing_if = "Option::is_none")]
    eth: Option<ProtocolData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snap: Option<ProtocolData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    p2p: Option<ProtocolData>,
}

/// Serializable peer protocol data returned by the node's rpc
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProtocolData {
    version: u32,
}

impl From<PeerData> for RpcPeer {
    fn from(peer: PeerData) -> Self {
        let mut protocols = Protocols::default();
        // Fill protocol data
        for cap in &peer.supported_capabilities {
            match cap {
                // TODO (https://github.com/lambdaclass/ethrex/issues/1578) Save the versions of each capability
                // For now we will be hardcoding our supported versions
                Capability::P2p => protocols.p2p = Some(ProtocolData { version: 5 }),
                Capability::Eth => protocols.eth = Some(ProtocolData { version: 68 }),
                Capability::Snap => protocols.snap = Some(ProtocolData { version: 1 }),
                // Ignore capabilities we don't know of
                Capability::UnsupportedCapability(_) => {}
            }
        }
        RpcPeer {
            caps: peer.supported_capabilities,
            enode: peer.node.enode_url(),
            id: peer.node.node_id(),
            network: PeerNetwork {
                remote_address: peer.node.udp_addr(),
                inbound: peer.is_connection_inbound,
            },
            protocols,
        }
    }
}

pub fn peers(context: &RpcApiContext) -> Result<Value, RpcErr> {
    let peers = context
        .peer_handler
        .read_connected_peers()
        .ok_or(RpcErr::Internal(String::from("Failed to read peers")))?
        .into_iter()
        .map(|peer| RpcPeer::from(peer))
        .collect::<Vec<_>>();
    Ok(serde_json::to_value(peers)?)
}

#[cfg(test)]
mod tests {
    use ethrex_p2p::types::{Node, NodeRecord};
    use k256::ecdsa::SigningKey;
    use rand::rngs::OsRng;
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_peer_data_to_serialized_peer() {
        // Test that we can correctly serialize an active Peer
        let node = Node::from_enode_url("enode://4aeb4ab6c14b23e2c4cfdce879c04b0748a20d8e9b59e25ded2a08143e265c6c25936e74cbc8e641e3312ca288673d91f2f93f8e277de3cfa444ecdaaf982052@157.90.35.166:30303").unwrap();
        let record = NodeRecord::from_node(&node, 17, &SigningKey::random(&mut OsRng)).unwrap();
        let mut peer = PeerData::new(node, record, true);
        // Set node capabilities and other relevant data
        peer.is_connected = true;
        peer.is_connection_inbound = false;
        peer.supported_capabilities = vec![Capability::Eth, Capability::Snap];
        // The first serialized peer shown in geth's documentation example: https://geth.ethereum.org/docs/interacting-with-geth/rpc/ns-admin#admin-peers
        // The fields "localAddress", "static", "trusted" and "name" were removed as we do not have the necessary information to show them
        // Also the capability versions were removed as we don't currenlty store them in the Capability enum
        // We should add them along with https://github.com/lambdaclass/ethrex/issues/1578
        // Misc: Added 0x prefix to node id, there is no set spec for this method so the prefix shouldn't be a problem
        let expected_serialized_peer = r#"{"caps":["eth","snap"],"enode":"enode://4aeb4ab6c14b23e2c4cfdce879c04b0748a20d8e9b59e25ded2a08143e265c6c25936e74cbc8e641e3312ca288673d91f2f93f8e277de3cfa444ecdaaf982052@157.90.35.166:30303","id":"0x6b36f791352f15eb3ec4f67787074ab8ad9d487e37c4401d383f0561a0a20507","network":{"inbound":false,"remoteAddress":"157.90.35.166:30303"},"protocols":{"eth":{"version":68},"snap":{"version":1}}}"#.to_string();
        let serialized_peer =
            serde_json::to_string(&RpcPeer::from(peer)).expect("Failed to serialize peer");
        assert_eq!(serialized_peer, expected_serialized_peer);
    }
}
