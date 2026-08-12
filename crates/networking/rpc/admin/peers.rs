use crate::utils::RpcRequest;
use crate::{rpc::RpcApiContext, utils::RpcErr};
use core::net::SocketAddr;
use ethrex_common::H256;
use ethrex_p2p::{
    peer_handler::PeerHandler,
    peer_table::PeerData,
    rlpx::{initiator::RlpxInitiatorProtocol as _, p2p::Capability},
    types::Node,
};
use serde::Serialize;
use serde_json::Value;
use tokio::time::{Duration, Instant};

/// Serializable peer data returned by the node's rpc
#[derive(Serialize)]
pub struct RpcPeer {
    caps: Vec<Capability>,
    enode: String,
    id: H256,
    name: String,
    network: PeerNetwork,
    protocols: Protocols,
}

/// Serializable peer network data returned by the node's rpc
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PeerNetwork {
    // We can add more data about the connection here, such the local address, whether the peer is trusted, etc
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
    /// `pbtsnap/1` — state sync for the EIP-8297 binary tree.
    ///
    /// Additive: the key is absent for every peer that does not advertise the
    /// capability, so a response from a peer set without it is byte-identical
    /// to what this method returned before. Without this arm a `pbtsnap` peer
    /// showed the capability only in the raw `caps` list, which is not where an
    /// operator looks to see what a peer speaks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pbtsnap: Option<ProtocolData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    p2p: Option<ProtocolData>,
}

/// Serializable peer protocol data returned by the node's rpc
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProtocolData {
    version: u8,
}

impl From<PeerData> for RpcPeer {
    fn from(peer: PeerData) -> Self {
        let mut protocols = Protocols::default();
        // Fill protocol data
        for cap in &peer.supported_capabilities {
            match cap.protocol() {
                "eth" => {
                    protocols.eth = Some(ProtocolData {
                        version: cap.version,
                    })
                }
                "snap" => {
                    protocols.snap = Some(ProtocolData {
                        version: cap.version,
                    })
                }
                "pbtsnap" => {
                    protocols.pbtsnap = Some(ProtocolData {
                        version: cap.version,
                    })
                }
                // Ignore capabilities we don't know of
                _ => {}
            }
        }
        RpcPeer {
            caps: peer.supported_capabilities,
            enode: peer.node.enode_url(),
            id: peer.node.node_id(),
            name: peer.node.version.clone().unwrap_or("Unknown".to_string()),
            network: PeerNetwork {
                remote_address: peer.node.udp_addr(),
                inbound: peer.is_connection_inbound,
            },
            protocols,
        }
    }
}

pub async fn peers(context: &mut RpcApiContext) -> Result<Value, RpcErr> {
    let Some(peer_handler) = &mut context.peer_handler else {
        return Err(RpcErr::Internal("Peer handler not initialized".to_string()));
    };

    let peers = peer_handler
        .read_connected_peers()
        .await
        .into_iter()
        .map(RpcPeer::from)
        .collect::<Vec<_>>();

    Ok(serde_json::to_value(peers)?)
}

fn parse(request: &RpcRequest) -> Result<Node, RpcErr> {
    let params = request
        .params
        .clone()
        .ok_or(RpcErr::MissingParam("enode url".to_string()))?;

    if params.len() != 1 {
        return Err(RpcErr::BadParams("Expected 1 param".to_owned()));
    };

    let url = params
        .first()
        .ok_or(RpcErr::MissingParam("enode url".to_string()))?
        .as_str()
        .ok_or(RpcErr::WrongParam("Expected string".to_string()))?;

    Node::from_enode_url(url).map_err(|error| RpcErr::BadParams(error.to_string()))
}

pub async fn add_peer(context: &mut RpcApiContext, request: &RpcRequest) -> Result<Value, RpcErr> {
    let Some(peer_handler) = context.peer_handler.as_mut() else {
        return Err(RpcErr::Internal("Peer handler not initialized".to_string()));
    };
    let server = peer_handler.initiator.clone();
    let node = parse(request)?;

    let start = Instant::now();
    let runtime = Duration::from_secs(10);

    let cast_result = server.initiate(node.clone());
    // This loop is necessary because connections are asynchronous, so to check if the connection with the peer was actually
    // established we need to wait.
    loop {
        if peer_is_connected(peer_handler, &node.enode_url()).await {
            return Ok(serde_json::to_value(true)?);
        }

        if cast_result.is_err() || start.elapsed() >= runtime {
            return Ok(serde_json::to_value(false)?);
        }
        let _ = tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn peer_is_connected(peer_handler: &mut PeerHandler, enode_url: &str) -> bool {
    peer_handler
        .read_connected_peers()
        .await
        .iter()
        .any(|peer| peer.node.enode_url() == *enode_url)
}

pub async fn peer_scores(context: &mut RpcApiContext) -> Result<Value, RpcErr> {
    let Some(peer_handler) = &context.peer_handler else {
        return Err(RpcErr::Internal("Peer handler not initialized".to_string()));
    };

    let diagnostics = peer_handler.read_peer_diagnostics().await;
    let total = diagnostics.len();
    let eligible = diagnostics.iter().filter(|p| p.eligible).count();
    let avg_score = if total > 0 {
        diagnostics.iter().map(|p| p.score).sum::<i64>() / total as i64
    } else {
        0
    };
    let total_inflight: i64 = diagnostics.iter().map(|p| p.inflight_requests).sum();

    let response = serde_json::json!({
        "peers": diagnostics,
        "summary": {
            "total_peers": total,
            "eligible_peers": eligible,
            "average_score": avg_score,
            "total_inflight_requests": total_inflight,
        }
    });

    Ok(response)
}

pub async fn sync_status(context: &mut RpcApiContext) -> Result<Value, RpcErr> {
    let Some(syncer) = &context.syncer else {
        return Err(RpcErr::Internal("Sync manager not initialized".to_string()));
    };

    let diag = syncer.get_sync_diagnostics().await;
    serde_json::to_value(diag).map_err(|e| RpcErr::Internal(e.to_string()))
}

// TODO: Adapt the test to the new P2P architecture.
#[cfg(test)]
mod tests {
    use ethrex_p2p::types::{Node, NodeRecord};
    use rand::rngs::OsRng;
    use secp256k1::SecretKey;

    use super::*;

    #[test]
    fn test_peer_data_to_serialized_peer() {
        // Test that we can correctly serialize an active Peer
        let node = Node::from_enode_url("enode://4aeb4ab6c14b23e2c4cfdce879c04b0748a20d8e9b59e25ded2a08143e265c6c25936e74cbc8e641e3312ca288673d91f2f93f8e277de3cfa444ecdaaf982052@157.90.35.166:30303").unwrap();
        let record = NodeRecord::from_node(&node, 17, &SecretKey::new(&mut OsRng)).unwrap();
        let mut peer = PeerData::new(
            node,
            Some(record),
            None,
            vec![Capability::eth(68), Capability::snap(1)],
        );
        // Set node capabilities and other relevant data
        peer.is_connection_inbound = false;
        peer.node.version = Some("ethrex/test".to_string());
        // The first serialized peer shown in geth's documentation example: https://geth.ethereum.org/docs/interacting-with-geth/rpc/ns-admin#admin-peers
        // The fields "localAddress", "static", "trusted" and "name" were removed as we do not have the necessary information to show them
        // Misc: Added 0x prefix to node id, there is no set spec for this method so the prefix shouldn't be a problem, also changed version name
        let expected_serialized_peer = r#"{"caps":["eth/68","snap/1"],"enode":"enode://4aeb4ab6c14b23e2c4cfdce879c04b0748a20d8e9b59e25ded2a08143e265c6c25936e74cbc8e641e3312ca288673d91f2f93f8e277de3cfa444ecdaaf982052@157.90.35.166:30303","id":"0x6b36f791352f15eb3ec4f67787074ab8ad9d487e37c4401d383f0561a0a20507","name":"ethrex/test","network":{"inbound":false,"remoteAddress":"157.90.35.166:30303"},"protocols":{"eth":{"version":68},"snap":{"version":1}}}"#.to_string();
        let serialized_peer =
            serde_json::to_string(&RpcPeer::from(peer)).expect("Failed to serialize peer");
        assert_eq!(serialized_peer, expected_serialized_peer);
    }

    /// A `pbtsnap/1` peer must show the capability where an operator reads what
    /// a peer speaks — the `protocols` object — and not only in the raw `caps`
    /// list.
    ///
    /// Observed on a live devnet before this arm existed: all three peers
    /// advertised `pbtsnap/1` in `caps` and `protocols` said nothing about it.
    #[test]
    fn a_pbtsnap_peer_reports_the_capability_under_protocols() {
        let node = Node::from_enode_url("enode://4aeb4ab6c14b23e2c4cfdce879c04b0748a20d8e9b59e25ded2a08143e265c6c25936e74cbc8e641e3312ca288673d91f2f93f8e277de3cfa444ecdaaf982052@157.90.35.166:30303").unwrap();
        let record = NodeRecord::from_node(&node, 17, &SecretKey::new(&mut OsRng)).unwrap();
        let mut peer = PeerData::new(
            node,
            Some(record),
            None,
            vec![
                Capability::eth(68),
                Capability::snap(1),
                Capability::pbtsnap(1),
            ],
        );
        peer.is_connection_inbound = false;
        peer.node.version = Some("ethrex/test".to_string());

        let serialized = serde_json::to_value(RpcPeer::from(peer)).expect("peer must serialize");

        // The capability is in `caps`, which is what was already true and is
        // not what this test is about. Asserting it establishes that the peer
        // really does advertise `pbtsnap/1`, so the check below is not passing
        // because the fixture forgot to.
        assert_eq!(
            serialized["caps"],
            serde_json::json!(["eth/68", "snap/1", "pbtsnap/1"]),
        );
        // The claim: it is under `protocols` too, at its negotiated version.
        assert_eq!(
            serialized["protocols"]["pbtsnap"],
            serde_json::json!({ "version": 1 }),
            "a pbtsnap peer must report the protocol, got {}",
            serialized["protocols"],
        );
        // And the pre-existing keys are untouched, so this is additive.
        assert_eq!(
            serialized["protocols"]["eth"],
            serde_json::json!({ "version": 68 }),
        );
        assert_eq!(
            serialized["protocols"]["snap"],
            serde_json::json!({ "version": 1 }),
        );
    }

    /// The other half of "additive": a peer that does not speak `pbtsnap` must
    /// serialize without the key at all, so the response shape a non-binary
    /// deployment sees is byte-identical to what it was before.
    ///
    /// `test_peer_data_to_serialized_peer` above pins that exact string; this
    /// states the invariant it depends on, so a later change from
    /// `Option<ProtocolData>` to something that renders `null` is caught here
    /// with a message that says why rather than only as a string diff.
    #[test]
    fn a_peer_without_pbtsnap_omits_the_key_entirely() {
        let node = Node::from_enode_url("enode://4aeb4ab6c14b23e2c4cfdce879c04b0748a20d8e9b59e25ded2a08143e265c6c25936e74cbc8e641e3312ca288673d91f2f93f8e277de3cfa444ecdaaf982052@157.90.35.166:30303").unwrap();
        let record = NodeRecord::from_node(&node, 17, &SecretKey::new(&mut OsRng)).unwrap();
        let mut peer = PeerData::new(
            node,
            Some(record),
            None,
            vec![Capability::eth(68), Capability::snap(1)],
        );
        peer.is_connection_inbound = false;
        peer.node.version = Some("ethrex/test".to_string());

        let serialized = serde_json::to_value(RpcPeer::from(peer)).expect("peer must serialize");

        assert!(
            serialized["protocols"].get("pbtsnap").is_none(),
            "a peer that does not advertise pbtsnap must not carry the key, got {}",
            serialized["protocols"],
        );
        // Non-vacuity: `protocols` is populated at all, so the assertion above
        // is not passing because the object is empty.
        assert_eq!(
            serialized["protocols"]["eth"],
            serde_json::json!({ "version": 68 }),
        );
    }
}
