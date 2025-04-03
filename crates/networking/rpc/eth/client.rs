use serde_json::Value;
use tracing::info;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

pub struct ChainId;
impl RpcHandler for ChainId {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        info!("Requested chain id");
        let chain_spec = context
            .storage
            .get_chain_config()
            .map_err(|error| RpcErr::Internal(error.to_string()))?;
        serde_json::to_value(format!("{:#x}", chain_spec.chain_id))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

pub struct Syncing;
impl RpcHandler for Syncing {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let is_synced = context.storage.is_synced()?;
        Ok(Value::Bool(!is_synced))
    }
}
