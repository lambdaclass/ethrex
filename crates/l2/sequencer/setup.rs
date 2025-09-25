use crate::sequencer::errors::ProofCoordinatorError;
use ethrex_common::types::TxType;
use ethrex_common::utils::keccak;
use ethrex_common::{Address, Bytes};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_common::utils::get_address_from_secret_key;
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_l2_sdk::{build_generic_tx, send_tx_bump_gas_exponential_backoff};
use ethrex_rpc::clients::{Overrides, eth::EthClient};
use secp256k1::SecretKey;
use std::str::FromStr;

use tracing::{debug, info};

use std::process::Command;

const QPL_TOOL_PATH: &str = "./tee/contracts/automata-dcap-qpl/automata-dcap-qpl-tool/target/release/automata-dcap-qpl-tool";

pub async fn prepare_quote_prerequisites(
    eth_client: &EthClient,
    rpc_url: &str,
    private_key_str: &str,
    quote: &str,
) -> Result<(), ProofCoordinatorError> {
    let chain_id = eth_client
        .get_chain_id()
        .await
        .map_err(ProofCoordinatorError::EthClientError)?;

    Command::new(QPL_TOOL_PATH)
        .args([
            "--chain_id",
            &chain_id.to_string(),
            "--rpc_url",
            rpc_url,
            "-p",
            private_key_str,
            "--quote_hex",
            quote,
        ])
        .env("RPC_URL", rpc_url)
        .env("CHAIN_ID", format!("{chain_id}"))
        .output()
        .map_err(ProofCoordinatorError::ComandError)?;
    Ok(())
}

const TDX_REGISTER_FUNCTION_SIGNATURE: &str = "register(bytes)";

pub async fn register_tdx_key(
    eth_client: &EthClient,
    private_key: &SecretKey,
    on_chain_proposer_address: Address,
    quote: Bytes,
) -> Result<(), ProofCoordinatorError> {
    debug!("Registering TDX key");

    let calldata_values = vec![Value::Bytes(quote)];

    let calldata = encode_calldata(TDX_REGISTER_FUNCTION_SIGNATURE, &calldata_values)?;

    let gas_price = eth_client
        .get_gas_price_with_extra(20)
        .await?
        .try_into()
        .map_err(|_| {
            ProofCoordinatorError::InternalError("Failed to convert gas_price to a u64".to_owned())
        })?;

    let tdx_address = get_tdx_address(eth_client, on_chain_proposer_address).await?;
    let verify_tx = build_generic_tx(
        eth_client,
        TxType::EIP1559,
        tdx_address,
        get_address_from_secret_key(private_key).map_err(ProofCoordinatorError::InternalError)?,
        calldata.into(),
        Overrides {
            max_fee_per_gas: Some(gas_price),
            max_priority_fee_per_gas: Some(gas_price),
            ..Default::default()
        },
    )
    .await?;

    let signer = Signer::Local(LocalSigner::new(*private_key));

    let verify_tx_hash =
        send_tx_bump_gas_exponential_backoff(eth_client, verify_tx, &signer).await?;

    info!("Registered TDX key with transaction hash {verify_tx_hash:#x}");
    Ok(())
}

async fn get_tdx_address(
    eth_client: &EthClient,
    on_chain_proposer_address: Address,
) -> Result<Address, ProofCoordinatorError> {
    let calldata = keccak("TDXVERIFIER()")[..4].to_vec();

    let response = eth_client
        .call(
            on_chain_proposer_address,
            calldata.into(),
            Overrides::default(),
        )
        .await?;
    // trim to 20 bytes, also removes 0x prefix
    let trimmed_response = &response[26..];

    Address::from_str(&format!("0x{trimmed_response}")).map_err(|_| {
        ProofCoordinatorError::InternalError(
            "Failed to convert TDXVERIFIER result to address".to_owned(),
        )
    })
}
