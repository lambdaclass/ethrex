use ethrex_common::serde_utils;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

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
        debug!("Requested chain id");
        let chain_spec = context
            .storage
            .get_chain_config()
            .map_err(|error| RpcErr::Internal(error.to_string()))?;
        serde_json::to_value(format!("{:#x}", chain_spec.chain_id))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

pub struct Syncing;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncingStatusRpc {
    #[serde(with = "serde_utils::u64::hex_str")]
    starting_block: u64,
    #[serde(with = "serde_utils::u64::hex_str")]
    current_block: u64,
    #[serde(with = "serde_utils::u64::hex_str")]
    highest_block: u64,
}

impl RpcHandler for Syncing {
    /// Ref: https://ethereum.org/en/developers/docs/apis/json-rpc/#eth_syncing
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        if context.blockchain.is_synced() {
            Ok(Value::Bool(!context.blockchain.is_synced()))
        } else {
            let syncing_status = SyncingStatusRpc {
                starting_block: context.storage.get_earliest_block_number().await?,
                current_block: context.storage.get_latest_block_number().await?,
                highest_block: context
                    .syncer
                    .get_last_fcu_head()
                    .map_err(|error| RpcErr::Internal(error.to_string()))?
                    .into_inner()
                    .to_low_u64_be(),
            };
            serde_json::to_value(syncing_status)
                .map_err(|error| RpcErr::Internal(error.to_string()))
        }
    }
}
