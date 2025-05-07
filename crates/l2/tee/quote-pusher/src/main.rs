use std::collections::HashMap;
use std::env;
use std::str::FromStr;

use ethereum_types::{Address, H160, H256, U256};
use ethrex_l2_sdk::calldata::{encode_calldata, Value};
use ethrex_l2_sdk::get_address_from_secret_key;
use ethrex_rpc::clients::eth::errors::{CalldataEncodeError, EthClientError};
use ethrex_rpc::clients::eth::EthClient;
use secp256k1::SecretKey;
use std::process::Command;


#[derive(Debug, thiserror::Error)]
pub enum PusherError {
    #[error("Missing env variable: {0}")]
    MissingConfig(String),
    #[error("Parsing Error: {0}")]
    ParseError(String),
    #[error("Request Error: {0}")]
    RequestError(reqwest::Error),
    #[error("Invalid request response, missing key: {0}")]
    ResponseMissingKey(String),
    #[error("Invalid request response, invalid value: {0}")]
    ResponseInvalidValue(String),
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("Deployer EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Command execution error: {0}")]
    CommandError(std::io::Error),
}

const UPDATE_KEY_SIGNATURE: &str = "updateKey(address,bytes)";

async fn setup_key(
    eth_client: &EthClient,
    rpc_url: &str,
    private_key_str: &str,
    web_client: &reqwest::Client,
    private_key: &SecretKey,
    prover_url: &str,
    contract_addr: Address,
) -> Result<(), PusherError> {
    let resp = web_client
        .get(format!("{prover_url}/getkey"))
        .send()
        .await
        .map_err(PusherError::RequestError)?;
    let json = resp
        .json::<HashMap<String, String>>()
        .await
        .map_err(|_| PusherError::ParseError("Couldn't parse getkey response".to_string()))?;

    let sig_addr = json
        .get("address")
        .ok_or(PusherError::ResponseMissingKey("address".to_string()))?;
    let quote = json
        .get("quote")
        .ok_or(PusherError::ResponseMissingKey("quote".to_string()))?;

    let chain_id = eth_client.get_chain_id().await.map_err(PusherError::EthClientError)?;

    Command::new("./automata-dcap-qpl/automata-dcap-qpl-tool/target/release/automata-dcap-qpl-tool").args([
        "--chain_id",
        &chain_id.to_string(),
        "--rpc_url",
        rpc_url,
        "-p",
        private_key_str,
        "--quote_hex",
        &quote
    ]).output().map_err(PusherError::CommandError)?;

    let sig_addr = H160::from_str(&sig_addr)
        .map_err(|_| PusherError::ResponseInvalidValue("Invalid address".to_string()))?;
    let quote = hex::decode(&quote)
        .map_err(|_| PusherError::ResponseInvalidValue("Invalid quote".to_string()))?;

    let my_address = get_address_from_secret_key(&private_key)
        .map_err(|_| PusherError::ParseError("Invalid private key".to_string()))?;

    let calldata = encode_calldata(
        UPDATE_KEY_SIGNATURE,
        &[Value::Address(sig_addr), Value::Bytes(quote.into())],
    )
    .map_err(PusherError::CalldataEncodeError)?;

    let tx = eth_client
        .build_eip1559_transaction(
            contract_addr,
            my_address,
            calldata.into(),
            Default::default(),
        )
        .await
        .map_err(PusherError::EthClientError)?;
    let mut wrapped_tx = ethrex_rpc::clients::eth::WrappedTransaction::EIP1559(tx);
    eth_client
        .set_gas_for_wrapped_tx(&mut wrapped_tx, my_address)
        .await
        .map_err(PusherError::EthClientError)?;
    let initialize_tx_hash = eth_client
        .send_tx_bump_gas_exponential_backoff(&mut wrapped_tx, &private_key)
        .await
        .map_err(PusherError::EthClientError)?;
    println!("Signing key set. TX: {initialize_tx_hash}");
    Ok(())
}

const UPDATE_SIGNATURE: &str = "update(address,bytes)";

async fn do_transition(
    eth_client: &EthClient,
    web_client: &reqwest::Client,
    private_key: &SecretKey,
    prover_url: &str,
    contract_addr: Address,
    state: u64,
) -> Result<u64, PusherError> {
    let map: HashMap<String, String> = HashMap::new();
    let resp = web_client
        .get(format!("{prover_url}/transition"))
        .query(&[("state", state)])
        .send()
        .await
        .map_err(PusherError::RequestError)?;
    let json = resp
        .json::<HashMap<String, String>>()
        .await
        .map_err(|_| PusherError::ParseError("Couldn't parse getkey response".to_string()))?;

    let new_state = json
        .get("address")
        .ok_or(PusherError::ResponseMissingKey("address".to_string()))?;
    let signature = json
        .get("quote")
        .ok_or(PusherError::ResponseMissingKey("quote".to_string()))?;

    let new_state = u64::from_str(&new_state)
        .map_err(|_| PusherError::ResponseInvalidValue("Invalid new_state".to_string()))?;
    let signature = hex::decode(&signature)
        .map_err(|_| PusherError::ResponseInvalidValue("Invalid signature".to_string()))?;

    let my_address = get_address_from_secret_key(&private_key)
        .map_err(|_| PusherError::ParseError("Invalid private key".to_string()))?;

    let calldata = encode_calldata(
        UPDATE_SIGNATURE,
        &[
            Value::Uint(U256([0, 0, 0, new_state])),
            Value::Bytes(signature.into()),
        ],
    )
    .map_err(PusherError::CalldataEncodeError)?;

    let tx = eth_client
        .build_eip1559_transaction(
            contract_addr,
            my_address,
            calldata.into(),
            Default::default(),
        )
        .await
        .map_err(PusherError::EthClientError)?;
    let mut wrapped_tx = ethrex_rpc::clients::eth::WrappedTransaction::EIP1559(tx);
    eth_client
        .set_gas_for_wrapped_tx(&mut wrapped_tx, my_address)
        .await
        .map_err(PusherError::EthClientError)?;
    let initialize_tx_hash = eth_client
        .send_tx_bump_gas_exponential_backoff(&mut wrapped_tx, &private_key)
        .await
        .map_err(PusherError::EthClientError)?;
    println!("Updated state. TX: {initialize_tx_hash}");
    Ok(new_state)
}

fn read_env_var(name: &str) -> Result<String, PusherError> {
    env::var(name.to_string()).map_err(|_| PusherError::MissingConfig(name.to_string()))
}

#[tokio::main]
async fn main() -> Result<(), PusherError> {
    let rpc_url = read_env_var("RPC_URL")?;
    let private_key_str = read_env_var("PRIVATE_KEY")?;
    let contract_addr = read_env_var("CONTRACT_ADDRESS")?;
    let prover_url = env::var("PROVER_URL").unwrap_or("http://localhost:3001".to_string());

    let private_key = SecretKey::from_slice(
        H256::from_str(&private_key_str)
            .map_err(|_| PusherError::ParseError("Invalid PRIVATE_KEY".to_string()))?
            .as_bytes(),
    )
    .map_err(|_| PusherError::ParseError("Invalid PRIVATE_KEY".to_string()))?;
    let contract_addr: Address = H160::from_str(&contract_addr)
        .map_err(|_| PusherError::ParseError("Invalid CONTRACT_ADDRESS".to_string()))?;

    let eth_client = EthClient::new(&rpc_url);
    let web_client = reqwest::Client::new();

    let mut state = 100;
    setup_key(
        &eth_client,
        &rpc_url,
        &private_key_str,
        &web_client,
        &private_key,
        &prover_url,
        contract_addr,
    )
    .await?;
    loop {
        state = do_transition(
            &eth_client,
            &web_client,
            &private_key,
            &prover_url,
            contract_addr,
            state,
        )
        .await?;
        println!("New state: {state}");
    }
}
