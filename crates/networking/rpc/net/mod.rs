use serde_json::Value;

use crate::{
    rpc::RpcApiContext,
    utils::{RpcErr, RpcRequest},
};

pub fn version(_req: &RpcRequest, context: RpcApiContext) -> Result<Value, RpcErr> {
    let chain_spec = context.storage.get_chain_config()?;

    let value = serde_json::to_value(format!("{}", chain_spec.chain_id))?;
    Ok(value)
}

// dummy function
pub fn peer_count(_req: &RpcRequest, _context: RpcApiContext) -> Result<Value, RpcErr> {
    let value = serde_json::to_value("0")?;
    Ok(value)
}
