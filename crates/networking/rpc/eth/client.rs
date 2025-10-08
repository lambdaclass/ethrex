use std::collections::BTreeMap;

use ethrex_common::H32;
use ethrex_common::H160;
use ethrex_common::serde_utils;
use ethrex_common::types::Fork;
use ethrex_common::types::ForkBlobSchedule;
use ethrex_common::types::ForkId;
use ethrex_vm::{precompiles_for_fork, system_contracts::system_contracts_for_fork};
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
                    .to_low_u64_be(),
            };
            serde_json::to_value(syncing_status)
                .map_err(|error| RpcErr::Internal(error.to_string()))
        }
    }
}

pub struct Config;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EthConfigObject {
    activation_time: Option<u64>,
    blob_schedule: Option<ForkBlobSchedule>,
    #[serde(with = "serde_utils::u64::hex_str")]
    chain_id: u64,
    fork_id: H32,
    precompiles: BTreeMap<String, H160>,
    system_contracts: BTreeMap<String, H160>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EthConfigResponse {
    current: EthConfigObject,
    next: Option<EthConfigObject>,
    last: Option<EthConfigObject>,
}

impl RpcHandler for Config {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let chain_config = context.storage.get_chain_config()?;
        let Some(latest_block) = context
            .storage
            .get_block_by_number(context.storage.get_latest_block_number().await?)
            .await?
        else {
            return Err(RpcErr::Internal("Failed to fetch latest block".to_string()));
        };

        let latest_block_timestamp = latest_block.header.timestamp;
        let current_fork = chain_config.get_fork(latest_block_timestamp);

        if current_fork < Fork::Paris {
            return Err(RpcErr::UnsuportedFork(
                "eth-config is not supported for forks prior to Paris".to_string(),
            ));
        }

        let current = get_config_for_fork(current_fork, &context).await?;
        let next = if let Some(next_fork) = chain_config.next_fork(latest_block_timestamp) {
            Some(get_config_for_fork(next_fork, &context).await?)
        } else {
            None
        };
        let last_fork = chain_config.get_last_scheduled_fork();
        let last = if last_fork > current_fork {
            Some(get_config_for_fork(last_fork, &context).await?)
        } else {
            None
        };
        let response = EthConfigResponse {
            current,
            next,
            last,
        };

        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

async fn get_config_for_fork(
    fork: Fork,
    context: &RpcApiContext,
) -> Result<EthConfigObject, RpcErr> {
    let chain_config = context.storage.get_chain_config()?;
    let activation_time = chain_config.get_activation_timestamp_for_fork(fork);
    let genesis_header = context
        .storage
        .get_block_by_number(0)
        .await?
        .expect("Failed to get genesis block. This should not happen.")
        .header;
    let block_number = context.storage.get_latest_block_number().await?;
    let fork_id = if let Some(timestamp) = activation_time {
        ForkId::new(chain_config, genesis_header, timestamp, block_number).fork_hash
    } else {
        H32::zero()
    };
    let mut system_contracts = BTreeMap::new();
    for contract in system_contracts_for_fork(fork) {
        system_contracts.insert(contract.name.to_string(), contract.address);
    }

    let mut precompiles = BTreeMap::new();

    for precompile in precompiles_for_fork(fork) {
        precompiles.insert(precompile.name.to_string(), precompile.address);
    }

    Ok(EthConfigObject {
        activation_time,
        blob_schedule: chain_config.get_blob_schedule_for_fork(fork),
        chain_id: chain_config.chain_id,
        fork_id,
        precompiles,
        system_contracts,
    })
}
