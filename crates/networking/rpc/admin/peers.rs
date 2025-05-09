use std::net::IpAddr;

use ethrex_common::H512;
use ethrex_p2p::{kademlia::PeerData, rlpx::p2p::Capability};
use serde::Serialize;
use serde_json::Value;

use crate::{rpc::RpcApiContext, utils::RpcErr};

/// Serializable peer data returned by the node's rpc
#[derive(Serialize)]
pub struct RpcPeer {
    caps: Vec<Capability>,
    enode: String,
    id: H512,
    network: PeerNetwork,
    protocols: Protocols,
}

/// Serializable peer network data returned by the node's rpc
#[derive(Serialize)]
struct PeerNetwork {
    remote_address: IpAddr, // We can add more data about the connection here, such as if it is inbound, the local address, etc
}

/// Serializable peer protocols data returned by the node's rpc
#[derive(Default, Serialize)]
struct Protocols {
    #[serde(skip_serializing_if = "Option::is_none")]
    snap: Option<ProtocolData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eth: Option<ProtocolData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    p2p: Option<ProtocolData>,
}

/// Serializable peer protocol data returned by the node's rpc
#[derive(Serialize)]
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
            id: peer.node.node_id,
            network: PeerNetwork {
                remote_address: peer.node.ip,
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
