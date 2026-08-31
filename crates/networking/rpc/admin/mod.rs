use ethrex_common::types::{BlockHash, ChainConfig};
use ethrex_storage::Store;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use tracing_subscriber::{EnvFilter, Registry, reload};

use crate::{
    rpc::NodeData,
    utils::{RpcErr, RpcRequest},
};
mod peers;
pub use peers::{add_peer, peer_scores, peers, sync_status};

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct NodeInfo {
    enode: String,
    enr: String,
    id: String,
    ip: String,
    listen_addr: String,
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
    Eth(EthProtocolInfo),
}

#[derive(Serialize, Debug)]
struct EthProtocolInfo {
    network: u64,
    genesis: BlockHash,
    config: ChainConfig,
    head: BlockHash,
}

pub async fn node_info(storage: Store, node_data: &NodeData) -> Result<Value, RpcErr> {
    // Read the live identity; clone out values and drop the guard before any .await.
    let (enode_url, enr_url, node_id, ip, udp_port, tcp_port) = {
        let guard = node_data
            .shared_local_node
            .read()
            .map_err(|_| RpcErr::Internal("shared_local_node lock poisoned".to_string()))?;
        let node = &guard.node;
        let enode_url = node.enode_url();
        let enr_url = guard.record.enr_url().unwrap_or_default();
        let node_id = hex::encode(node.node_id());
        let ip = node.ip.to_string();
        let udp_port = node.udp_port;
        let tcp_port = node.tcp_port;
        (enode_url, enr_url, node_id, ip, udp_port, tcp_port)
    };

    let chain_config = storage.get_chain_config();

    let genesis_hash = storage
        .get_block_header(0)
        .map_err(|e| RpcErr::Internal(e.to_string()))?
        .map(|h| h.hash())
        .unwrap_or_default();

    let head_hash = storage
        .get_latest_canonical_block_hash()
        .await
        .map_err(|e| RpcErr::Internal(e.to_string()))?
        .unwrap_or_default();

    let eth_info = EthProtocolInfo {
        network: chain_config.chain_id,
        genesis: genesis_hash,
        config: chain_config,
        head: head_hash,
    };

    let mut protocols = HashMap::new();
    protocols.insert("eth".to_string(), Protocol::Eth(eth_info));

    let node_info = NodeInfo {
        enode: enode_url,
        enr: enr_url,
        id: node_id,
        ip: ip.clone(),
        listen_addr: format!("{ip}:{tcp_port}"),
        name: node_data.client_version.to_string(),
        ports: Ports {
            discovery: udp_port,
            listener: tcp_port,
        },
        protocols,
    };
    serde_json::to_value(node_info).map_err(|error| RpcErr::Internal(error.to_string()))
}

pub fn set_log_level(
    req: &RpcRequest,
    log_filter_handler: &Option<reload::Handle<EnvFilter, Registry>>,
) -> Result<Value, RpcErr> {
    let params = req
        .params
        .clone()
        .ok_or(RpcErr::MissingParam("log level".to_string()))?;
    let log_level = params
        .first()
        .ok_or(RpcErr::MissingParam("log level".to_string()))?
        .as_str()
        .ok_or(RpcErr::WrongParam("Expected string".to_string()))?;

    let filter = EnvFilter::try_new(log_level)
        .map_err(|_| RpcErr::BadParams(format!("Cannot parse {log_level} as a log directive")))?;

    if let Some(handle) = log_filter_handler {
        handle
            .reload(filter)
            .map_err(|e| RpcErr::Internal(format!("Failed to reload log filter: {}", e)))?;
        Ok(Value::Bool(true))
    } else {
        Err(RpcErr::Internal(
            "Log filter handler not available".to_string(),
        ))
    }
}
