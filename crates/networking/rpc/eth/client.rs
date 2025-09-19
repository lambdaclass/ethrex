use std::collections::BTreeMap;

use ethrex_common::types::Fork;
use ethrex_common::types::precompile::precompiles_for_fork;
use ethrex_common::{H32, addresses::*};
use ethrex_common::{H160, serde_utils, types::ForkBlobSchedule};
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
    activation_time: u64,
    blob_schedule: ForkBlobSchedule,
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
    last: Option<EthConfigObject>,
    next: Option<EthConfigObject>,
}

impl RpcHandler for Config {
    fn parse(_params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        Ok(Self {})
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let latest_block_number = context.storage.get_latest_block_number().await?;
        let chain_config = context.storage.get_chain_config()?;
        let fork_id = context.storage.get_fork_id().await?;
        let mut system_contracts = BTreeMap::new();
        system_contracts.insert("BECON_ROOTS_ADDRESS".to_string(), *BEACON_ROOTS_ADDRESS);
        if chain_config.fork(latest_block_number) >= Fork::Prague {
            system_contracts.insert(
                "CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS".to_string(),
                *CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
            );
            system_contracts.insert(
                "DEPOSIT_CONTRACT_ADDRESS".to_string(),
                chain_config.deposit_contract_address,
            );
            system_contracts.insert(
                "HISTORY_STORAGE_ADDRESS".to_string(),
                *HISTORY_STORAGE_ADDRESS,
            );
            system_contracts.insert(
                "WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS".to_string(),
                *WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
            );
        }

        let mut precompiles = BTreeMap::new();

        for precompile in precompiles_for_fork(chain_config.get_fork(latest_block_number)) {
            precompiles.insert(precompile.name.to_string(), precompile.address);
        }

        let current = EthConfigObject {
            activation_time: chain_config
                .get_current_fork_activation_timestamp(latest_block_number),
            blob_schedule: chain_config
                .get_fork_blob_schedule(latest_block_number)
                .unwrap_or_default(),
            chain_id: chain_config.chain_id,
            fork_id: fork_id.fork_hash,
            precompiles,
            system_contracts,
        };

        let response = EthConfigResponse {
            current,
            last: None,
            next: None,
        };

        serde_json::to_value(response).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
