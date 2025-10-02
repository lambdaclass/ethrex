use aligned_sdk::common::types::Network;
use ethrex_common::utils::keccak;
use ethrex_common::{Address, H160, H256, types::TxType};
use ethrex_l2_common::prover::ProverType;
use ethrex_l2_rpc::signer::Signer;
use ethrex_l2_sdk::{
    build_generic_tx, get_last_committed_batch, get_last_verified_batch,
    send_tx_bump_gas_exponential_backoff,
};
use ethrex_rpc::{
    EthClient,
    clients::{EthClientError, Overrides},
};
use ethrex_storage_rollup::{RollupStoreError, StoreRollup};
use rand::Rng;
use std::time::{Duration, SystemTime};
use std::{str::FromStr, time::UNIX_EPOCH};
use tokio::time::sleep;
use tracing::info;

const DEV_MODE_ADDRESS: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xAA,
]);

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

pub fn system_now_ms() -> Option<u128> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis())
}

pub async fn send_verify_tx(
    encoded_calldata: Vec<u8>,
    eth_client: &EthClient,
    on_chain_proposer_address: Address,
    l1_signer: &Signer,
) -> Result<H256, EthClientError> {
    let gas_price = eth_client
        .get_gas_price_with_extra(20)
        .await?
        .try_into()
        .map_err(|_| {
            EthClientError::InternalError("Failed to convert gas_price to a u64".to_owned())
        })?;

    let verify_tx = build_generic_tx(
        eth_client,
        TxType::EIP1559,
        on_chain_proposer_address,
        l1_signer.address(),
        encoded_calldata.into(),
        Overrides {
            max_fee_per_gas: Some(gas_price),
            max_priority_fee_per_gas: Some(gas_price),
            ..Default::default()
        },
    )
    .await?;

    let verify_tx_hash =
        send_tx_bump_gas_exponential_backoff(eth_client, verify_tx, l1_signer).await?;

    Ok(verify_tx_hash)
}

pub async fn get_needed_proof_types(
    rpc_urls: Vec<String>,
    on_chain_proposer_address: Address,
) -> Result<Vec<ProverType>, EthClientError> {
    let eth_client = EthClient::new_with_multiple_urls(rpc_urls)?;

    let mut needed_proof_types = vec![];
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
                "Failed to parse OnChainProposer response {response}"
            ))
        })?;

        if address != DEV_MODE_ADDRESS {
            info!("{prover_type} proof needed");
            needed_proof_types.push(prover_type);
        }
    }
    if needed_proof_types.is_empty() {
        needed_proof_types.push(ProverType::Exec);
    }

    Ok(needed_proof_types)
}

pub async fn get_latest_sent_batch(
    needed_proof_types: Vec<ProverType>,
    rollup_store: &StoreRollup,
    eth_client: &EthClient,
    on_chain_proposer_address: Address,
) -> Result<u64, SequencerError> {
    if needed_proof_types.contains(&ProverType::Aligned) {
        Ok(rollup_store.get_lastest_sent_batch_proof().await?)
    } else {
        Ok(get_last_verified_batch(eth_client, on_chain_proposer_address).await?)
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
    let last_committed_batch_number =
        get_last_committed_batch(eth_client, on_chain_proposer_address).await?;

    let is_up_to_date = rollup_storage
        .contains_batch(&last_committed_batch_number)
        .await?;

    Ok(is_up_to_date)
}
