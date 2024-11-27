use ethereum_types::{Address, H160, H256, U256};
use ethrex_core::types::{PrivilegedTxType, Transaction};
use ethrex_l2::utils::{
    eth_client::{
        errors::{EthClientError, GetTransactionReceiptError},
        eth_sender::Overrides,
        EthClient,
    },
    merkle_tree::merkle_proof,
};
use ethrex_rpc::types::{block::BlockBodyWrapper, receipt::RpcReceipt};
use itertools::Itertools;
use keccak_hash::keccak;
use secp256k1::SecretKey;

// 0x6bf26397c5676a208d5c4e5f35cb479bacbbe454
pub const DEFAULT_BRIDGE_ADDRESS: Address = H160([
    0x6b, 0xf2, 0x63, 0x97, 0xc5, 0x67, 0x6a, 0x20, 0x8d, 0x5c, 0x4e, 0x5f, 0x35, 0xcb, 0x47, 0x9b,
    0xac, 0xbb, 0xe4, 0x54,
]);

#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    #[error("Failed to parse address from hex")]
    FailedToParseAddressFromHex,
}

/// BRIDGE_ADDRESS or 0x6bf26397c5676a208d5c4e5f35cb479bacbbe454
pub fn bridge_address() -> Result<Address, SdkError> {
    std::env::var("BRIDGE_ADDRESS")
        .unwrap_or(format!("{DEFAULT_BRIDGE_ADDRESS:#x}"))
        .parse()
        .map_err(|_| SdkError::FailedToParseAddressFromHex)
}

pub async fn wait_for_transaction_receipt(
    tx_hash: H256,
    client: &EthClient,
    max_retries: u64,
) -> Option<RpcReceipt> {
    let mut receipt = client
        .get_transaction_receipt(tx_hash)
        .await
        .expect("Failed to get transaction receipt");
    let mut r#try = 1;
    while receipt.is_none() {
        println!("[{try}/{max_retries}] Retrying to get transaction receipt for {tx_hash:#x}");

        if max_retries == r#try {
            panic!("Transaction receipt for {tx_hash:#x} not found after {max_retries} retries");
        }
        r#try += 1;

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        receipt = client
            .get_transaction_receipt(tx_hash)
            .await
            .expect("Failed to get transaction receipt");
    }
    receipt
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
        .build_privileged_transaction(
            PrivilegedTxType::Withdrawal,
            from,
            from,
            Default::default(),
            Overrides {
                value: Some(amount),
                gas_price: Some(800000000),
                gas_limit: Some(21000 * 2),
                ..Default::default()
            },
            10,
        )
        .await?;

    proposer_client
        .send_privileged_l2_transaction(&withdraw_transaction, &from_pk)
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

    let claim_withdrawal_data = {
        let mut calldata = Vec::new();

        // Function selector
        calldata.extend_from_slice(&keccak(CLAIM_WITHDRAWAL_SIGNATURE).as_bytes()[..4]);

        // bytes32 l2WithdrawalTxHash
        calldata.extend_from_slice(l2_withdrawal_tx_hash.as_fixed_bytes());

        // uint256 claimedAmount
        let mut encoded_amount = [0; 32];
        claimed_amount.to_big_endian(&mut encoded_amount);
        calldata.extend_from_slice(&encoded_amount);

        // uint256 withdrawalBlockNumber
        let mut encoded_block_number = [0; 32];
        withdrawal_l2_block_number.to_big_endian(&mut encoded_block_number);
        calldata.extend_from_slice(&encoded_block_number);

        // uint256 withdrawalLogIndex
        let mut encoded_idx = [0; 32];
        U256::from(index).to_big_endian(&mut encoded_idx);
        calldata.extend_from_slice(&encoded_idx);

        // bytes32[] withdrawalProof
        let mut encoded_offset = [0; 32];
        U256::from(32 * 5).to_big_endian(&mut encoded_offset);
        calldata.extend_from_slice(&encoded_offset);
        let mut encoded_proof_len = [0; 32];
        U256::from(proof.len()).to_big_endian(&mut encoded_proof_len);
        calldata.extend_from_slice(&encoded_proof_len);
        for hash in proof {
            calldata.extend_from_slice(hash.as_fixed_bytes());
        }

        calldata
    };

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
        .filter(|tx| match &tx.tx {
            Transaction::PrivilegedL2Transaction(tx) => tx.tx_type == PrivilegedTxType::Withdrawal,
            _ => false,
        })
        .find_position(|tx| tx.hash == tx_hash)
        .map(|(i, tx)| match &tx.tx {
            Transaction::PrivilegedL2Transaction(privileged_l2_transaction) => {
                privileged_l2_transaction
                    .get_withdrawal_hash()
                    .map(|withdrawal_hash| (i as u64, (withdrawal_hash)))
            }
            _ => unreachable!(),
        })
    else {
        return Err(EthClientError::Custom(
            "Failed to get widthdrawal hash, transaction is not a withdrawal".to_string(),
        ));
    };

    let path = merkle_proof(
        transactions
            .iter()
            .filter_map(|tx| match &tx.tx {
                Transaction::PrivilegedL2Transaction(tx) => tx.get_withdrawal_hash(),
                _ => None,
            })
            .collect(),
        tx_withdrawal_hash,
    )
    .ok_or(EthClientError::Custom(
        "Failed to generate merkle proof, element is not on the tree".to_string(),
    ))?;

    Ok((index, path))
}
