use serde_json::{Value, json};

use crate::{
    rpc::RpcApiContext,
    utils::{RpcErr, RpcRequest},
};

pub fn version(_req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    let chain_spec = context.storage.get_chain_config();

    let value = serde_json::to_value(format!("{}", chain_spec.chain_id))?;
    Ok(value)
}

pub async fn peer_count(_req: &RpcRequest, mut context: RpcApiContext) -> Result<Value, RpcErr> {
    let Some(peer_handler) = &mut context.peer_handler else {
        return Err(RpcErr::Internal("Peer handler not initialized".to_string()));
    };
    let total_peers = peer_handler
        .count_total_peers()
        .await
        .map_err(|e| RpcErr::Internal(format!("Could not retrieve peer count: {e}")))?;

    Ok(json!(format!("{:#x}", total_peers)))
}
