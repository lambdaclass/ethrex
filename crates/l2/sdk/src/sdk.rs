use bytes::Bytes;
use calldata::{encode_calldata, Value};
use ethereum_types::{Address, H160, H256, U256};
use ethrex_common::types::{Transaction, TxKind};
use ethrex_rpc::clients::eth::{
    errors::{EthClientError, GetTransactionReceiptError},
    eth_sender::Overrides,
    EthClient,
};
use ethrex_rpc::types::{block::BlockBodyWrapper, receipt::RpcReceipt};
use itertools::Itertools;
use keccak_hash::keccak;
use merkle_tree::merkle_proof;
use secp256k1::SecretKey;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
pub mod calldata;
pub mod merkle_tree;

// 0x6bf26397c5676a208d5c4e5f35cb479bacbbe454
pub const DEFAULT_BRIDGE_ADDRESS: Address = H160([
    0x6b, 0xf2, 0x63, 0x97, 0xc5, 0x67, 0x6a, 0x20, 0x8d, 0x5c, 0x4e, 0x5f, 0x35, 0xcb, 0x47, 0x9b,
    0xac, 0xbb, 0xe4, 0x54,
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

/// BRIDGE_ADDRESS or 0x6bf26397c5676a208d5c4e5f35cb479bacbbe454
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
    private_key: SecretKey,
    client: &EthClient,
) -> Result<H256, EthClientError> {
    println!(
        "Transferring {amount} from {from:#x} to {to:#x}",
        amount = amount,
        from = from,
        to = to
    );
    let tx = client
        .build_eip1559_transaction(
            to,
            from,
            Default::default(),
            Overrides {
                value: Some(amount),
                ..Default::default()
            },
            10,
        )
        .await?;
    client.send_eip1559_transaction(&tx, &private_key).await
}

pub async fn deposit(
    amount: U256,
    from: Address,
    from_pk: SecretKey,
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
            10,
        )
        .await?;

    proposer_client
        .send_eip1559_transaction(&withdraw_transaction, &from_pk)
        .await
}

pub async fn claim_withdraw(
    l2_withdrawal_tx_hash: H256,
    amount: U256,
    from: Address,
    from_pk: SecretKey,
    proposer_client: &EthClient,
    eth_client: &EthClient,
) -> Result<H256, EthClientError> {
    println!("Claiming {amount} from bridge to {from:#x}");

    const CLAIM_WITHDRAWAL_SIGNATURE: &str =
        "claimWithdrawal(bytes32,uint256,uint256,uint256,bytes32[])";

    let (withdrawal_l2_block_number, claimed_amount) = match proposer_client
        .get_transaction_by_hash(l2_withdrawal_tx_hash)
        .await?
    {
        Some(l2_withdrawal_tx) => (l2_withdrawal_tx.block_number, l2_withdrawal_tx.value),
        None => {
            println!("Withdrawal transaction not found in L2");
            return Err(EthClientError::GetTransactionReceiptError(
                GetTransactionReceiptError::RPCError(
                    "Withdrawal transaction not found in L2".to_owned(),
                ),
            ));
        }
    };

    let (index, proof) = get_withdraw_merkle_proof(proposer_client, l2_withdrawal_tx_hash).await?;

    let calldata_values = vec![
        Value::Uint(U256::from_big_endian(
            l2_withdrawal_tx_hash.as_fixed_bytes(),
        )),
        Value::Uint(claimed_amount),
        Value::Uint(withdrawal_l2_block_number),
        Value::Uint(U256::from(index)),
        Value::Array(
            proof
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
            10,
        )
        .await?;

    eth_client
        .send_eip1559_transaction(&claim_tx, &from_pk)
        .await
}

/// Returns the formated hash of the withdrawal transaction,
/// or None if the transaction is not a withdrawal.
/// The hash is computed as keccak256(to || value || tx_hash)
pub fn get_withdrawal_hash(tx: &Transaction) -> Option<H256> {
    let to_bytes: [u8; 20] = match tx.data().get(16..36)?.try_into() {
        Ok(value) => value,
        Err(_) => return None,
    };
    let to = Address::from(to_bytes);

    let value = tx.value().to_big_endian();

    Some(keccak_hash::keccak(
        [to.as_bytes(), &value, tx.compute_hash().as_bytes()].concat(),
    ))
}

pub async fn get_withdraw_merkle_proof(
    client: &EthClient,
    tx_hash: H256,
) -> Result<(u64, Vec<H256>), EthClientError> {
    let tx_receipt =
        client
            .get_transaction_receipt(tx_hash)
            .await?
            .ok_or(EthClientError::Custom(
                "Failed to get transaction receipt".to_string(),
            ))?;

    let block = client
        .get_block_by_hash(tx_receipt.block_info.block_hash)
        .await?;

    let transactions = match block.body {
        BlockBodyWrapper::Full(body) => body.transactions,
        BlockBodyWrapper::OnlyHashes(_) => unreachable!(),
    };
    let Some(Some((index, tx_withdrawal_hash))) = transactions
        .iter()
        .filter(|tx| match &tx.tx.to() {
            ethrex_common::types::TxKind::Call(to) => *to == COMMON_BRIDGE_L2_ADDRESS,
            ethrex_common::types::TxKind::Create => false,
        })
        .find_position(|tx| tx.hash == tx_hash)
        .map(|(i, tx)| get_withdrawal_hash(&tx.tx).map(|withdrawal_hash| (i, (withdrawal_hash))))
    else {
        return Err(EthClientError::Custom(
            "Failed to get widthdrawal hash, transaction is not a withdrawal".to_string(),
        ));
    };

    let path = merkle_proof(
        transactions
            .iter()
            .filter_map(|tx| match tx.tx.to() {
                TxKind::Call(to) if to == COMMON_BRIDGE_L2_ADDRESS => get_withdrawal_hash(&tx.tx),
                _ => None,
            })
            .collect(),
        tx_withdrawal_hash,
    )
    .map_err(|err| EthClientError::Custom(format!("Failed to generate merkle proof: {err}")))?
    .ok_or(EthClientError::Custom(
        "Failed to generate merkle proof, element is not on the tree".to_string(),
    ))?;

    Ok((
        index
            .try_into()
            .map_err(|err| EthClientError::Custom(format!("index does not fit in u64: {}", err)))?,
        path,
    ))
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
