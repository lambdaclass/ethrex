use bytes::Bytes;
use calldata::{encode_calldata, Value};
use ethereum_types::{Address, H160, H256, U256};
use ethrex_common::types::signer::LocalSigner;
use ethrex_common::types::GenericTransaction;
use ethrex_rpc::clients::eth::{
    errors::EthClientError, eth_sender::Overrides, EthClient, WithdrawalProof,
};
use ethrex_rpc::types::receipt::RpcReceipt;

use keccak_hash::keccak;
use secp256k1::SecretKey;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod calldata;
pub mod l1_to_l2_tx_data;
pub mod merkle_tree;

pub use l1_to_l2_tx_data::{send_l1_to_l2_tx, L1ToL2TransactionData};

// 0x554a14cd047c485b3ac3edbd9fbb373d6f84ad3f
pub const DEFAULT_BRIDGE_ADDRESS: Address = H160([
    0x55, 0x4a, 0x14, 0xcd, 0x04, 0x7c, 0x48, 0x5b, 0x3a, 0xc3, 0xed, 0xbd, 0x9f, 0xbb, 0x37, 0x3d,
    0x6f, 0x84, 0xad, 0x3f,
]);

pub const COMMON_BRIDGE_L2_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xff,
]);

pub const L2_WITHDRAW_SIGNATURE: &str = "withdraw(address)";

#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    #[error("Failed to parse address from hex")]
    FailedToParseAddressFromHex,
}

/// BRIDGE_ADDRESS or 0x554a14cd047c485b3ac3edbd9fbb373d6f84ad3f
pub fn bridge_address() -> Result<Address, SdkError> {
    std::env::var("L1_WATCHER_BRIDGE_ADDRESS")
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
    let signer = LocalSigner::new(*private_key).into();
    client.send_eip1559_transaction(&tx, &signer).await
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
                // CHECK: If we don't set max_fee_per_gas and max_priority_fee_per_gas
                // The transaction is not included on the L2.
                // Also we have some mismatches at the end of the L2 integration test.
                max_fee_per_gas: Some(800000000),
                max_priority_fee_per_gas: Some(800000000),
                gas_limit: Some(21000 * 2),
                ..Default::default()
            },
        )
        .await?;

    let signer = LocalSigner::new(from_pk).into();
    proposer_client
        .send_eip1559_transaction(&withdraw_transaction, &signer)
        .await
}

pub async fn claim_withdraw(
    amount: U256,
    l2_withdrawal_tx_hash: H256,
    from: Address,
    from_pk: SecretKey,
    eth_client: &EthClient,
    withdrawal_proof: &WithdrawalProof,
) -> Result<H256, EthClientError> {
    println!("Claiming {amount} from bridge to {from:#x}");

    const CLAIM_WITHDRAWAL_SIGNATURE: &str =
        "claimWithdrawal(bytes32,uint256,uint256,uint256,bytes32[])";

    let calldata_values = vec![
        Value::Uint(U256::from_big_endian(
            l2_withdrawal_tx_hash.as_fixed_bytes(),
        )),
        Value::Uint(amount),
        Value::Uint(withdrawal_proof.batch_number.into()),
        Value::Uint(U256::from(withdrawal_proof.index)),
        Value::Array(
            withdrawal_proof
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

    let signer = LocalSigner::new(from_pk).into();
    eth_client
        .send_eip1559_transaction(&claim_tx, &signer)
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
