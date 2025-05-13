use ethrex_common::types::ChainConfig;
use ethrex_storage::Store;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::{rpc::NodeData, utils::RpcErr};

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

pub fn node_info(storage: Store, node_data: &NodeData) -> Result<Value, RpcErr> {
    let enode_url = node_data.local_p2p_node.enode_url();
    let enr_url = match node_data.local_node_record.enr_url() {
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
        id: hex::encode(node_data.local_p2p_node.node_id()),
        name: node_data.client_version.clone(),
        ip: node_data.local_p2p_node.ip.to_string(),
        ports: Ports {
            discovery: node_data.local_p2p_node.udp_port,
            listener: node_data.local_p2p_node.tcp_port,
        },
        protocols,
    };
    serde_json::to_value(node_info).map_err(|error| RpcErr::Internal(error.to_string()))
}
