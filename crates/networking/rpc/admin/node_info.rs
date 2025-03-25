use ethrex_common::types::ChainConfig;
use ethrex_p2p::types::{Node, NodeRecord};
use ethrex_storage::Store;
use serde::Serialize;
use serde_json::Value;
use sha3::{Digest, Keccak256};
use std::collections::HashMap;

use crate::context::RpcApiContext;
use crate::rpc_types::RpcErr;
use crate::server::RpcHandler;

#[derive(Serialize, Debug)]
struct NodeInfo {
    enode: String,
    enr: String,
    id: String,
    ip: String,
    name: String,
    ports: Ports,
    protocols: HashMap<String, Protocol>,
}

#[derive(Serialize, Debug)]
struct Ports {
    discovery: u16,
    listener: u16,
}

#[derive(Serialize, Debug)]
#[serde(untagged)]
enum Protocol {
    Eth(ChainConfig),
}

pub struct NodeInfoRequest;

impl RpcHandler for NodeInfoRequest {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(NodeInfoRequest)
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        node_info(
            context.storage,
            context.local_p2p_node,
            context.local_node_record,
        )
    }
}

// Keeping this function for compatibility with existing code
pub fn node_info(
    storage: Store,
    local_node: Node,
    local_node_record: NodeRecord,
) -> Result<Value, RpcErr> {
    let enode_url = local_node.enode_url();
    let enr_url = match local_node_record.enr_url() {
        Ok(enr) => enr,
        Err(_) => "".into(),
    };
    let mut protocols = HashMap::new();

    let chain_config = storage
        .get_chain_config()
        .map_err(|error| RpcErr::Internal(error.to_string()))?;
    protocols.insert("eth".to_string(), Protocol::Eth(chain_config));

    let node_info = NodeInfo {
        enode: enode_url,
        enr: enr_url,
        id: hex::encode(Keccak256::digest(local_node.node_id.as_bytes())),
        name: "ethrex/0.1.0/rust1.82".to_string(),
        ip: local_node.ip.to_string(),
        ports: Ports {
            discovery: local_node.udp_port,
            listener: local_node.tcp_port,
        },
        protocols,
    };
    serde_json::to_value(node_info).map_err(|error| RpcErr::Internal(error.to_string()))
}
