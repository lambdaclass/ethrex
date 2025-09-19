use serde_json::{Value, json};

use crate::{
    rpc::RpcApiContext,
    utils::{RpcErr, RpcRequest},
};

pub fn version(_req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    let chain_spec = context.storage.get_chain_config()?;

    let value = serde_json::to_value(format!("{}", chain_spec.chain_id))?;
    Ok(value)
}

pub async fn peer_count(_req: &RpcRequest, mut context: RpcApiContext) -> Result<Value, RpcErr> {
    let total_peers = context.peer_handler.count_total_peers().await;

    Ok(json!(format!("{:#x}", total_peers)))
}
