use std::{fs::read_to_string, path::Path, process::Command};

use bytes::Bytes;
use calldata::{Value, encode_calldata};
use ethereum_types::{Address, H160, H256, U256};
use ethrex_common::types::GenericTransaction;
use ethrex_rpc::clients::eth::L1MessageProof;
use ethrex_rpc::clients::eth::{
    EthClient, WrappedTransaction, errors::EthClientError, eth_sender::Overrides,
};
use ethrex_rpc::types::receipt::RpcReceipt;

use keccak_hash::keccak;
use secp256k1::SecretKey;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod calldata;
pub mod l1_to_l2_tx_data;
pub mod merkle_tree;

pub use l1_to_l2_tx_data::{L1ToL2TransactionData, send_l1_to_l2_tx};

// 0x8ccf74999c496e4d27a2b02941673f41dd0dab2a
pub const DEFAULT_BRIDGE_ADDRESS: Address = H160([
    0x8c, 0xcf, 0x74, 0x99, 0x9c, 0x49, 0x6e, 0x4d, 0x27, 0xa2, 0xb0, 0x29, 0x41, 0x67, 0x3f, 0x41,
    0xdd, 0x0d, 0xab, 0x2a,
]);

pub const COMMON_BRIDGE_L2_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xff,
]);

pub const L1_MESSENGER_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfe,
]);

pub const L2_WITHDRAW_SIGNATURE: &str = "withdraw(address)";

#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    #[error("Failed to parse address from hex")]
    FailedToParseAddressFromHex,
}

/// BRIDGE_ADDRESS or 0x554a14cd047c485b3ac3edbd9fbb373d6f84ad3f
pub fn bridge_address() -> Result<Address, SdkError> {
    std::env::var("ETHREX_WATCHER_BRIDGE_ADDRESS")
        .unwrap_or(format!("{DEFAULT_BRIDGE_ADDRESS:#x}"))
        .parse()
        .map_err(|_| SdkError::FailedToParseAddressFromHex)
}

pub async fn wait_for_transaction_receipt(
    tx_hash: H256,
    client: &EthClient,
    max_retries: u64,
) -> Result<RpcReceipt, EthClientError> {
    let mut receipt = client.get_transaction_receipt(tx_hash).await?;
    let mut r#try = 1;
    while receipt.is_none() {
        println!("[{try}/{max_retries}] Retrying to get transaction receipt for {tx_hash:#x}");

        if max_retries == r#try {
            return Err(EthClientError::Custom(format!(
                "Transaction receipt for {tx_hash:#x} not found after {max_retries} retries"
            )));
        }
        r#try += 1;

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        receipt = client.get_transaction_receipt(tx_hash).await?;
    }
    receipt.ok_or(EthClientError::Custom(
        "Transaction receipt is None".to_owned(),
    ))
}

pub async fn transfer(
    amount: U256,
    from: Address,
    to: Address,
    private_key: &SecretKey,
    client: &EthClient,
) -> Result<H256, EthClientError> {
    println!(
        "Transferring {amount} from {from:#x} to {to:#x}",
        amount = amount,
        from = from,
        to = to
    );
    let gas_price = client
        .get_gas_price_with_extra(20)
        .await?
        .try_into()
        .map_err(|_| {
            EthClientError::InternalError("Failed to convert gas_price to a u64".to_owned())
        })?;

    let mut tx = client
        .build_eip1559_transaction(
            to,
            from,
            Default::default(),
            Overrides {
                value: Some(amount),
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?;

    let mut tx_generic: GenericTransaction = tx.clone().into();
    tx_generic.from = from;
    let gas_limit = client.estimate_gas(tx_generic).await?;
    tx.gas_limit = gas_limit;
    client.send_eip1559_transaction(&tx, private_key).await
}

pub async fn deposit_through_transfer(
    amount: U256,
    from: Address,
    from_pk: &SecretKey,
    eth_client: &EthClient,
) -> Result<H256, EthClientError> {
    println!("Depositing {amount} from {from:#x} to bridge");
    transfer(
        amount,
        from,
        bridge_address().map_err(|err| EthClientError::Custom(err.to_string()))?,
        from_pk,
        eth_client,
    )
    .await
}

pub async fn deposit_through_contract_call(
    amount: impl Into<U256>,
    to: Address,
    l1_gas_limit: u64,
    l2_gas_limit: u64,
    depositor_private_key: &SecretKey,
    bridge_address: Address,
    eth_client: &EthClient,
) -> Result<H256, EthClientError> {
    let l1_from = get_address_from_secret_key(depositor_private_key)?;
    send_l1_to_l2_tx(
        l1_from,
        Some(amount),
        Some(l1_gas_limit),
        L1ToL2TransactionData::new_deposit_data(to, l2_gas_limit),
        depositor_private_key,
        bridge_address,
        eth_client,
    )
    .await
}

pub async fn withdraw(
    amount: U256,
    from: Address,
    from_pk: SecretKey,
    proposer_client: &EthClient,
) -> Result<H256, EthClientError> {
    let withdraw_transaction = proposer_client
        .build_eip1559_transaction(
            COMMON_BRIDGE_L2_ADDRESS,
            from,
            Bytes::from(encode_calldata(
                L2_WITHDRAW_SIGNATURE,
                &[Value::Address(from)],
            )?),
            Overrides {
                value: Some(amount),
                ..Default::default()
            },
        )
        .await?;

    proposer_client
        .send_eip1559_transaction(&withdraw_transaction, &from_pk)
        .await
}

pub async fn claim_withdraw(
    amount: U256,
    l2_withdrawal_tx_hash: H256,
    from: Address,
    from_pk: SecretKey,
    eth_client: &EthClient,
    message_proof: &L1MessageProof,
) -> Result<H256, EthClientError> {
    println!("Claiming {amount} from bridge to {from:#x}");

    const CLAIM_WITHDRAWAL_SIGNATURE: &str =
        "claimWithdrawal(bytes32,uint256,uint256,uint256,bytes32[])";

    let calldata_values = vec![
        Value::Uint(U256::from_big_endian(
            l2_withdrawal_tx_hash.as_fixed_bytes(),
        )),
        Value::Uint(amount),
        Value::Uint(message_proof.batch_number.into()),
        Value::Uint(U256::from(message_proof.index)),
        Value::Array(
            message_proof
                .merkle_proof
                .iter()
                .map(|hash| Value::FixedBytes(hash.as_fixed_bytes().to_vec().into()))
                .collect(),
        ),
    ];

    let claim_withdrawal_data = encode_calldata(CLAIM_WITHDRAWAL_SIGNATURE, &calldata_values)?;

    println!(
        "Claiming withdrawal with calldata: {}",
        hex::encode(&claim_withdrawal_data)
    );

    let claim_tx = eth_client
        .build_eip1559_transaction(
            bridge_address().map_err(|err| EthClientError::Custom(err.to_string()))?,
            from,
            claim_withdrawal_data.into(),
            Overrides {
                from: Some(from),
                ..Default::default()
            },
        )
        .await?;

    eth_client
        .send_eip1559_transaction(&claim_tx, &from_pk)
        .await
}

pub fn secret_key_deserializer<'de, D>(deserializer: D) -> Result<SecretKey, D::Error>
where
    D: Deserializer<'de>,
{
    let hex = H256::deserialize(deserializer)?;
    SecretKey::from_slice(hex.as_bytes()).map_err(serde::de::Error::custom)
}

pub fn secret_key_serializer<S>(secret_key: &SecretKey, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let hex = H256::from_slice(&secret_key.secret_bytes());
    hex.serialize(serializer)
}

pub fn get_address_from_secret_key(secret_key: &SecretKey) -> Result<Address, EthClientError> {
    let public_key = secret_key
        .public_key(secp256k1::SECP256K1)
        .serialize_uncompressed();
    let hash = keccak(&public_key[1..]);

    // Get the last 20 bytes of the hash
    let address_bytes: [u8; 20] = hash
        .as_ref()
        .get(12..32)
        .ok_or(EthClientError::Custom(
            "Failed to get_address_from_secret_key: error slicing address_bytes".to_owned(),
        ))?
        .try_into()
        .map_err(|err| {
            EthClientError::Custom(format!("Failed to get_address_from_secret_key: {err}"))
        })?;

    Ok(Address::from(address_bytes))
}

#[derive(Debug, thiserror::Error)]
pub enum ContractCompilationError {
    #[error("The path is not a valid utf-8 string")]
    FailedToGetStringFromPath,
    #[error("Deployer compilation error: {0}")]
    CompilationError(String),
    #[error("Could not read file")]
    FailedToReadFile(#[from] std::io::Error),
    #[error("Failed to serialize/deserialize")]
    SerializationError(#[from] serde_json::Error),
}

pub fn compile_contract(
    general_contracts_path: &Path,
    contract_path: &str,
    runtime_bin: bool,
) -> Result<(), ContractCompilationError> {
    let bin_flag = if runtime_bin {
        "--bin-runtime"
    } else {
        "--bin"
    };

    // Both the contract path and the output path are relative to where the Makefile is.
    if !Command::new("solc")
        .arg(bin_flag)
        .arg(
            "@openzeppelin/contracts=".to_string()
                + general_contracts_path
                    .join("lib")
                    .join("openzeppelin-contracts-upgradeable")
                    .join("lib")
                    .join("openzeppelin-contracts")
                    .join("contracts")
                    .to_str()
                    .ok_or(ContractCompilationError::FailedToGetStringFromPath)?,
        )
        .arg(
            "@openzeppelin/contracts-upgradeable=".to_string()
                + general_contracts_path
                    .join("lib")
                    .join("openzeppelin-contracts-upgradeable")
                    .join("contracts")
                    .to_str()
                    .ok_or(ContractCompilationError::FailedToGetStringFromPath)?,
        )
        .arg(
            general_contracts_path
                .join(contract_path)
                .to_str()
                .ok_or(ContractCompilationError::FailedToGetStringFromPath)?,
        )
        .arg("--via-ir")
        .arg("-o")
        .arg(
            general_contracts_path
                .join("solc_out")
                .to_str()
                .ok_or(ContractCompilationError::FailedToGetStringFromPath)?,
        )
        .arg("--overwrite")
        .arg("--allow-paths")
        .arg(
            general_contracts_path
                .to_str()
                .ok_or(ContractCompilationError::FailedToGetStringFromPath)?,
        )
        .spawn()
        .map_err(|err| {
            ContractCompilationError::CompilationError(format!("Failed to spawn solc: {err}"))
        })?
        .wait()
        .map_err(|err| {
            ContractCompilationError::CompilationError(format!("Failed to wait for solc: {err}"))
        })?
        .success()
    {
        return Err(ContractCompilationError::CompilationError(
            format!("Failed to compile {contract_path}").to_owned(),
        ));
    }

    Ok(())
}

// 0x4e59b44847b379578588920cA78FbF26c0B4956C
const DETERMINISTIC_CREATE2_ADDRESS: Address = H160([
    0x4e, 0x59, 0xb4, 0x48, 0x47, 0xb3, 0x79, 0x57, 0x85, 0x88, 0x92, 0x0c, 0xa7, 0x8f, 0xbf, 0x26,
    0xc0, 0xb4, 0x95, 0x6c,
]);

#[derive(Default)]
pub struct ProxyDeployment {
    pub proxy_address: Address,
    pub proxy_tx_hash: H256,
    pub implementation_address: Address,
    pub implementation_tx_hash: H256,
}

#[derive(Debug, thiserror::Error)]
pub enum DeployError {
    #[error("Failed to decode init code: {0}")]
    FailedToReadInitCode(#[from] std::io::Error),
    #[error("Failed to decode init code: {0}")]
    FailedToDecodeBytecode(#[from] hex::FromHexError),
    #[error("Failed to deploy contract: {0}")]
    FailedToDeploy(#[from] EthClientError),
}

pub async fn deploy_contract(
    constructor_args: &[u8],
    contract_path: &Path,
    deployer_private_key: &SecretKey,
    salt: &[u8],
    eth_client: &EthClient,
) -> Result<(H256, Address), DeployError> {
    let bytecode = hex::decode(read_to_string(contract_path)?)?;
    let init_code = [&bytecode, constructor_args].concat();
    let (deploy_tx_hash, contract_address) =
        create2_deploy(salt, &init_code, deployer_private_key, eth_client).await?;
    Ok((deploy_tx_hash, contract_address))
}

async fn deploy_proxy(
    deployer_private_key: SecretKey,
    eth_client: &EthClient,
    contract_binaries: &Path,
    implementation_address: Address,
    salt: &[u8],
) -> Result<(H256, Address), DeployError> {
    let mut init_code = hex::decode(
        std::fs::read_to_string(contract_binaries.join("ERC1967Proxy.bin"))
            .map_err(DeployError::FailedToReadInitCode)?,
    )
    .map_err(DeployError::FailedToDecodeBytecode)?;

    init_code.extend(H256::from(implementation_address).0);
    init_code.extend(H256::from_low_u64_be(0x40).0);
    init_code.extend(H256::zero().0);

    let (deploy_tx_hash, proxy_address) = create2_deploy(
        salt,
        &Bytes::from(init_code),
        &deployer_private_key,
        eth_client,
    )
    .await
    .map_err(DeployError::from)?;

    Ok((deploy_tx_hash, proxy_address))
}

pub async fn deploy_with_proxy(
    deployer_private_key: SecretKey,
    eth_client: &EthClient,
    contract_binaries: &Path,
    contract_name: &str,
    salt: &[u8],
) -> Result<ProxyDeployment, DeployError> {
    let (implementation_tx_hash, implementation_address) = deploy_contract(
        &[],
        &contract_binaries.join(contract_name),
        &deployer_private_key,
        salt,
        eth_client,
    )
    .await?;

    let (proxy_tx_hash, proxy_address) = deploy_proxy(
        deployer_private_key,
        eth_client,
        contract_binaries,
        implementation_address,
        salt,
    )
    .await?;

    Ok(ProxyDeployment {
        proxy_address,
        proxy_tx_hash,
        implementation_address,
        implementation_tx_hash,
    })
}

async fn create2_deploy(
    salt: &[u8],
    init_code: &[u8],
    deployer_private_key: &SecretKey,
    eth_client: &EthClient,
) -> Result<(H256, Address), EthClientError> {
    let calldata = [salt, init_code].concat();
    let gas_price = eth_client
        .get_gas_price_with_extra(20)
        .await?
        .try_into()
        .map_err(|_| {
            EthClientError::InternalError("Failed to convert gas_price to a u64".to_owned())
        })?;

    let deployer_address = get_address_from_secret_key(deployer_private_key)?;

    let deploy_tx = eth_client
        .build_eip1559_transaction(
            DETERMINISTIC_CREATE2_ADDRESS,
            deployer_address,
            calldata.into(),
            Overrides {
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?;

    let mut wrapped_tx = ethrex_rpc::clients::eth::WrappedTransaction::EIP1559(deploy_tx);
    eth_client
        .set_gas_for_wrapped_tx(&mut wrapped_tx, deployer_address)
        .await?;
    let deploy_tx_hash = eth_client
        .send_tx_bump_gas_exponential_backoff(&mut wrapped_tx, deployer_private_key)
        .await?;

    wait_for_transaction_receipt(deploy_tx_hash, eth_client, 10).await?;

    let deployed_address = create2_address(salt, keccak(init_code));

    Ok((deploy_tx_hash, deployed_address))
}

#[allow(clippy::indexing_slicing)]
fn create2_address(salt: &[u8], init_code_hash: H256) -> Address {
    Address::from_slice(
        &keccak(
            [
                &[0xff],
                DETERMINISTIC_CREATE2_ADDRESS.as_bytes(),
                salt,
                init_code_hash.as_bytes(),
            ]
            .concat(),
        )
        .as_bytes()[12..],
    )
}

pub async fn initialize_contract(
    contract_address: Address,
    initialize_calldata: Vec<u8>,
    initializer_private_key: &SecretKey,
    eth_client: &EthClient,
) -> Result<H256, EthClientError> {
    let initializer_address = get_address_from_secret_key(initializer_private_key)?;

    let gas_price = eth_client
        .get_gas_price_with_extra(20)
        .await?
        .try_into()
        .map_err(|_| {
            EthClientError::InternalError("Failed to convert gas_price to a u64".to_owned())
        })?;

    let initialize_tx = eth_client
        .build_eip1559_transaction(
            contract_address,
            initializer_address,
            initialize_calldata.into(),
            Overrides {
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?;

    let mut wrapped_tx = WrappedTransaction::EIP1559(initialize_tx);

    eth_client
        .set_gas_for_wrapped_tx(&mut wrapped_tx, initializer_address)
        .await?;

    let initialize_tx_hash = eth_client
        .send_tx_bump_gas_exponential_backoff(&mut wrapped_tx, initializer_private_key)
        .await?;

    Ok(initialize_tx_hash)
}

pub async fn call_contract(
    client: &EthClient,
    private_key: &SecretKey,
    to: Address,
    signature: &str,
    parameters: Vec<Value>,
) -> Result<H256, EthClientError> {
    let calldata = encode_calldata(signature, &parameters)?.into();
    let from = get_address_from_secret_key(private_key)?;
    let tx = client
        .build_eip1559_transaction(to, from, calldata, Default::default())
        .await?;

    let tx_hash = client.send_eip1559_transaction(&tx, private_key).await?;

    wait_for_transaction_receipt(tx_hash, client, 100).await?;
    Ok(tx_hash)
}
