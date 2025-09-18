use std::collections::BTreeMap;

use ethrex_common::types::Fork;
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
struct ConfigRpcResponse {
    activation_time: u64,
    blob_schedule: ForkBlobSchedule,
    #[serde(with = "serde_utils::u64::hex_str")]
    chain_id: u64,
    fork_id: H32,
    precompiles: BTreeMap<String, H160>,
    system_contracts: BTreeMap<String, H160>,
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

        precompiles.insert("ECREC".to_string(), ECRECOVER_ADDRESS);
        precompiles.insert("SHA256".to_string(), SHA2_256_ADDRESS);
        precompiles.insert("RIPEMD160".to_string(), RIPEMD_160_ADDRESS);
        precompiles.insert("ID".to_string(), IDENTITY_ADDRESS);
        precompiles.insert("MODEXP".to_string(), MODEXP_ADDRESS);
        precompiles.insert("BN254_ADD".to_string(), ECADD_ADDRESS);
        precompiles.insert("BN254_MUL".to_string(), ECMUL_ADDRESS);
        precompiles.insert("BN254_PAIRING".to_string(), ECPAIRING_ADDRESS);
        precompiles.insert("BLAKE2F".to_string(), BLAKE2F_ADDRESS);
        precompiles.insert("KZG_POINT_EVALUATION".to_string(), POINT_EVALUATION_ADDRESS);

        if chain_config.fork(latest_block_number) >= Fork::Cancun {
            precompiles.insert("BLS12_G1ADD".to_string(), BLS12_G1ADD_ADDRESS);
            precompiles.insert("BLS12_G1MSM".to_string(), BLS12_G1MSM_ADDRESS);
            precompiles.insert("BLS12_G2ADD".to_string(), BLS12_G2ADD_ADDRESS);
            precompiles.insert("BLS12_G2MSM".to_string(), BLS12_G2MSM_ADDRESS);
            precompiles.insert(
                "BLS12_PAIRING_CHECK".to_string(),
                BLS12_PAIRING_CHECK_ADDRESS,
            );
            precompiles.insert("BLS12_MAP_FP_TO_G1".to_string(), BLS12_MAP_FP_TO_G1_ADDRESS);
            precompiles.insert(
                "BLS12_MAP_FP2_TO_G2".to_string(),
                BLS12_MAP_FP2_TO_G2_ADDRESS,
            );
        }

        if chain_config.fork(latest_block_number) >= Fork::Osaka {
            precompiles.insert("P256_VERIFY".to_string(), P256_VERIFICATION_ADDRESS);
        }
        let config = ConfigRpcResponse {
            activation_time: 0,
            blob_schedule: chain_config
                .get_fork_blob_schedule(latest_block_number)
                .unwrap_or_default(),
            chain_id: chain_config.chain_id,
            fork_id: fork_id.fork_hash,
            precompiles,
            system_contracts,
        };
        serde_json::to_value(config).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
