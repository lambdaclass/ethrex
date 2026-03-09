use aligned_sdk::types::Network;
use ethrex_common::types::Block;
use ethrex_common::types::batch::Batch;
use ethrex_common::types::fee_config::FeeConfig;
use ethrex_common::utils::keccak;
use ethrex_common::{Address, H256, types::TxType};
use ethrex_l2_common::prover::ProverType;
use ethrex_l2_rpc::signer::Signer;
use ethrex_l2_sdk::{
    build_generic_tx, get_last_committed_batch, send_tx_bump_gas_exponential_backoff,
};
use ethrex_rpc::{
    EthClient,
    clients::{EthClientError, Overrides},
};
use ethrex_storage::Store;
use ethrex_storage::error::StoreError;
use ethrex_storage_rollup::{RollupStoreError, StoreRollup};
use rand::Rng;
use reqwest::Url;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;
use tracing::{info, warn};

/// 4-byte ABI selectors for OnChainProposer custom errors.
/// These are the first 4 bytes of keccak256 of the error signature.
/// Used to detect specific revert reasons from the `data` field of RPC error responses.
pub const INVALID_RISC0_PROOF_SELECTOR: &str = "0x14add973";
pub const INVALID_SP1_PROOF_SELECTOR: &str = "0x7ff849b5";
pub const INVALID_TDX_PROOF_SELECTOR: &str = "0x62013a95";
pub const ALIGNED_PROOF_VERIFICATION_FAILED_SELECTOR: &str = "0x44602025";

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
    rpc_urls: Vec<Url>,
    on_chain_proposer_address: Address,
) -> Result<Vec<ProverType>, EthClientError> {
    let eth_client = EthClient::new_with_multiple_urls(rpc_urls)?;

    let mut needed_proof_types = vec![];
    for prover_type in ProverType::all() {
        let Some(getter) = prover_type.verifier_getter() else {
            continue;
        };
        let calldata = keccak(getter)[..4].to_vec();

        // response is a boolean 0x00..01 or 0x00..00
        let response = eth_client
            .call(
                on_chain_proposer_address,
                calldata.into(),
                Overrides::default(),
            )
            .await?;

        let required_proof_type = response
            .chars()
            .last()
            .ok_or(EthClientError::InternalError("empty response".to_string()))?
            == '1';
        if required_proof_type {
            info!("{prover_type} proof needed");
            needed_proof_types.push(prover_type);
        }
    }
    if needed_proof_types.is_empty() {
        needed_proof_types.push(ProverType::Exec);
    }

    Ok(needed_proof_types)
}

pub fn resolve_aligned_network(network: &str) -> Network {
    match network {
        "devnet" => Network::Devnet,
        "hoodi" => Network::Hoodi,
        "mainnet" => Network::Mainnet,
        unknown => {
            warn!(
                "Unknown Aligned network '{}', defaulting to devnet. Valid options: devnet, hoodi, mainnet",
                unknown
            );
            Network::Devnet
        }
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

pub async fn fetch_blocks_with_respective_fee_configs<E>(
    batch: &Batch,
    store: &Store,
    rollup_store: &StoreRollup,
) -> Result<(Vec<Block>, Vec<FeeConfig>), E>
where
    E: From<StoreError> + From<RollupStoreError>,
{
    let mut blocks = Vec::new();
    let mut fee_configs = vec![];

    for block_number in batch.first_block..=batch.last_block {
        let block_header = store
            .get_block_header(block_number)?
            .ok_or(StoreError::Custom(
                "failed to retrieve block header from storage".to_string(),
            ))?;

        let block_body = store
            .get_block_body(block_number)
            .await?
            .ok_or(StoreError::Custom(
                "failed to retrieve block body from storage".to_string(),
            ))?;

        let block = Block::new(block_header, block_body);

        blocks.push(block);

        // Fetch the L1 fee config for this block
        let fee_config = rollup_store
            .get_fee_config_by_block(block_number)
            .await?
            .ok_or(RollupStoreError::Custom("Fee config not found".to_string()))?;

        fee_configs.push(fee_config);
    }

    Ok((blocks, fee_configs))
}

/// Returns the git commit hash of the current build.
pub fn get_git_commit_hash() -> String {
    env!("VERGEN_GIT_SHA").to_string()
}

pub fn batch_checkpoint_name(batch_number: u64) -> String {
    format!("checkpoint_batch_{batch_number}")
}
