use aligned_sdk::common::types::Network;
use ethrex_common::{Address, H160, H256};
use ethrex_rpc::{
    EthClient,
    clients::{EthClientError, Overrides, eth::WrappedTransaction},
};
use ethrex_storage_rollup::{RollupStoreError, StoreRollup};
use keccak_hash::keccak;
use rand::Rng;
use secp256k1::SecretKey;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;
use tracing::info;

const DEV_MODE_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xAA,
]);

use crate::utils::prover::proving_systems::ProverType;

use super::errors::SequencerError;

pub async fn sleep_random(sleep_amount: u64) {
    sleep(random_duration(sleep_amount)).await;
}

pub fn random_duration(sleep_amount: u64) -> Duration {
    let random_noise: u64 = {
        let mut rng = rand::thread_rng();
        rng.gen_range(0..400)
    };
    Duration::from_millis(sleep_amount + random_noise)
}

pub async fn send_verify_tx(
    encoded_calldata: Vec<u8>,
    eth_client: &EthClient,
    on_chain_proposer_address: Address,
    l1_address: Address,
    l1_private_key: &SecretKey,
) -> Result<H256, EthClientError> {
    let gas_price = eth_client
        .get_gas_price_with_extra(20)
        .await?
        .try_into()
        .map_err(|_| {
            EthClientError::InternalError("Failed to convert gas_price to a u64".to_owned())
        })?;

    let verify_tx = eth_client
        .build_eip1559_transaction(
            on_chain_proposer_address,
            l1_address,
            encoded_calldata.into(),
            Overrides {
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?;

    let mut tx = WrappedTransaction::EIP1559(verify_tx);

    let verify_tx_hash = eth_client
        .send_tx_bump_gas_exponential_backoff(&mut tx, l1_private_key)
        .await?;

    Ok(verify_tx_hash)
}

pub async fn get_needed_proof_types(
    dev_mode: bool,
    rpc_urls: Vec<String>,
    on_chain_proposer_address: Address,
) -> Result<Vec<ProverType>, EthClientError> {
    let eth_client = EthClient::new_with_multiple_urls(rpc_urls)?;

    let mut needed_proof_types = vec![];
    if !dev_mode {
        for prover_type in ProverType::all() {
            let Some(getter) = prover_type.verifier_getter() else {
                continue;
            };
            let calldata = keccak(getter)[..4].to_vec();

            let response = eth_client
                .call(
                    on_chain_proposer_address,
                    calldata.into(),
                    Overrides::default(),
                )
                .await?;
            // trim to 20 bytes, also removes 0x prefix
            let trimmed_response = &response[26..];

            let address = Address::from_str(&format!("0x{trimmed_response}")).map_err(|_| {
                EthClientError::Custom(format!(
                    "Failed to parse OnChainProposer response {}",
                    response
                ))
            })?;

            if address != DEV_MODE_ADDRESS {
                info!("{prover_type} proof needed");
                needed_proof_types.push(prover_type);
            }
        }
    } else {
        needed_proof_types.push(ProverType::Exec);
    }
    Ok(needed_proof_types)
}

pub async fn get_latest_sent_batch(
    needed_proof_types: Vec<ProverType>,
    rollup_storage: &StoreRollup,
    eth_client: &EthClient,
    on_chain_proposer_address: Address,
) -> Result<u64, SequencerError> {
    if needed_proof_types.contains(&ProverType::Aligned) {
        Ok(rollup_storage.get_lastest_sent_batch_proof().await?)
    } else {
        Ok(eth_client
            .get_last_verified_batch(on_chain_proposer_address)
            .await?)
    }
}

pub fn resolve_aligned_network(network: &str) -> Network {
    match network {
        "devnet" => Network::Devnet,
        "holesky" => Network::Holesky,
        "holesky-stage" => Network::HoleskyStage,
        "mainnet" => Network::Mainnet,
        _ => Network::Devnet, // TODO: Implement custom networks
    }
}

pub async fn node_is_up_to_date<E>(
    eth_client: &EthClient,
    on_chain_proposer_address: Address,
    rollup_storage: &StoreRollup,
) -> Result<bool, E>
where
    E: From<EthClientError> + From<RollupStoreError>,
{
    let last_committed_batch_number = eth_client
        .get_last_committed_batch(on_chain_proposer_address)
        .await?;

    let is_up_to_date = rollup_storage
        .contains_batch(&last_committed_batch_number)
        .await?;

    Ok(is_up_to_date)
}
