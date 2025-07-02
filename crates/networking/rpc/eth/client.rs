use serde_json::{Map, Value, json};
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

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
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
    /// Ref: https://ethereum.org/en/developers/docs/apis/json-rpc/#eth_syncing
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        if context.blockchain.is_synced() {
            Ok(Value::Bool(!context.blockchain.is_synced()))
        } else {
            let mut map = Map::new();
            map.insert(
                "startingBlock".to_string(),
                json!(format!(
                    "{:#x}",
                    context.storage.get_earliest_block_number().await?
                )),
            );
            map.insert(
                "currentBlock".to_string(),
                json!(format!(
                    "{:#x}",
                    context.storage.get_latest_block_number().await?
                )),
            );
            map.insert(
                "highestBlock".to_string(),
                json!(format!(
                    "{:#x}",
                    context
                        .syncer
                        .get_last_fcu_head()
                        .try_lock()
                        .map_err(|error| RpcErr::Internal(error.to_string()))?
                        .to_low_u64_ne()
                )),
            );
            serde_json::to_value(map).map_err(|error| RpcErr::Internal(error.to_string()))
        }
    }
}
