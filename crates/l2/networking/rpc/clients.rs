use ethrex_common::{
    types::{EIP1559Transaction, TxKind, TxType, WrappedEIP4844Transaction},
    Address, H256, U256
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::{
    types::block_identifier::{BlockIdentifier, BlockTag},
    clients::{eth::{EthClient, WrappedTransaction}, EthClientError, Overrides,}
};
use crate::signer::{Signer, Signable};
use tracing::warn;
use bytes::Bytes;
use keccak_hash::keccak;

const WAIT_TIME_FOR_RECEIPT_SECONDS: u64 = 2;

pub async fn send_eip1559_transaction(
    client: &EthClient,
    tx: &EIP1559Transaction,
    signer: &Signer,
) -> Result<H256, EthClientError> {
    let signed_tx = tx.sign(signer).await.map_err(|err| EthClientError::Custom(err.to_string()))?;

    let mut encoded_tx = signed_tx.encode_to_vec();
    encoded_tx.insert(0, TxType::EIP1559.into());

    client.send_raw_transaction(encoded_tx.as_slice()).await
}

pub async fn send_eip4844_transaction(
    client: &EthClient,
    wrapped_tx: &WrappedEIP4844Transaction,
    signer: &Signer,
) -> Result<H256, EthClientError> {
    let mut wrapped_tx = wrapped_tx.clone();
    wrapped_tx.tx.sign_inplace(signer).await.map_err(|err| EthClientError::Custom(err.to_string()))?;

    let mut encoded_tx = wrapped_tx.encode_to_vec();
    encoded_tx.insert(0, TxType::EIP4844.into());

    client.send_raw_transaction(encoded_tx.as_slice()).await
}

pub async fn send_wrapped_transaction(
    client: &EthClient,
    wrapped_tx: &WrappedTransaction,
    signer: &Signer,
) -> Result<H256, EthClientError> {
    match wrapped_tx {
        WrappedTransaction::EIP4844(wrapped_eip4844_transaction) => {
            send_eip4844_transaction(client, wrapped_eip4844_transaction, signer)
                .await
        }
        WrappedTransaction::EIP1559(eip1559_transaction) => {
            send_eip1559_transaction(client, eip1559_transaction, signer)
                .await
        }
        WrappedTransaction::L2(privileged_l2_transaction) => {
            client.send_privileged_l2_transaction(privileged_l2_transaction)
                .await
        }
    }
}

pub async fn deploy(
    client: &EthClient,
    deployer: &Signer,
    init_code: Bytes,
    overrides: Overrides,
) -> Result<(H256, Address), EthClientError> {
    let mut deploy_overrides = overrides;
    deploy_overrides.to = Some(TxKind::Create);

    let deploy_tx = client
        .build_eip1559_transaction(
            Address::zero(),
            deployer.address(),
            init_code,
            deploy_overrides,
        )
        .await?;
    let deploy_tx_hash = send_eip1559_transaction(client, &deploy_tx, deployer).await?;

    let nonce = client
        .get_nonce(deployer.address(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;
    let mut encode = vec![];
    (deployer.address(), nonce).encode(&mut encode);

    //Taking the last 20bytes so it matches an H160 == Address length
    let deployed_address =
        Address::from_slice(keccak(encode).as_fixed_bytes().get(12..).ok_or(
            EthClientError::Custom("Failed to get deployed_address".to_owned()),
        )?);

    client.wait_for_transaction_receipt(deploy_tx_hash, 1000)
        .await?;

    Ok((deploy_tx_hash, deployed_address))
}

pub async fn send_tx_bump_gas_exponential_backoff(
    client: &EthClient,
    wrapped_tx: &mut WrappedTransaction,
    signer: &Signer,
) -> Result<H256, EthClientError> {
    let mut number_of_retries = 0;

    'outer: while number_of_retries < client.max_number_of_retries {
        if let Some(max_fee_per_gas) = client.maximum_allowed_max_fee_per_gas {
            let (tx_max_fee, tx_max_priority_fee) = match wrapped_tx {
                WrappedTransaction::EIP4844(tx) => (
                    &mut tx.tx.max_fee_per_gas,
                    &mut tx.tx.max_priority_fee_per_gas,
                ),
                WrappedTransaction::EIP1559(tx) => {
                    (&mut tx.max_fee_per_gas, &mut tx.max_priority_fee_per_gas)
                }
                WrappedTransaction::L2(tx) => {
                    (&mut tx.max_fee_per_gas, &mut tx.max_priority_fee_per_gas)
                }
            };

            if *tx_max_fee > max_fee_per_gas {
                *tx_max_fee = max_fee_per_gas;

                // Ensure that max_priority_fee_per_gas does not exceed max_fee_per_gas
                if *tx_max_priority_fee > *tx_max_fee {
                    *tx_max_priority_fee = *tx_max_fee;
                }

                warn!(
                    "max_fee_per_gas exceeds the allowed limit, adjusting it to {max_fee_per_gas}"
                );
            }
        }

        // Check blob gas fees only for EIP4844 transactions
        if let WrappedTransaction::EIP4844(tx) = wrapped_tx {
            if let Some(max_fee_per_blob_gas) = client.maximum_allowed_max_fee_per_blob_gas {
                if tx.tx.max_fee_per_blob_gas > U256::from(max_fee_per_blob_gas) {
                    tx.tx.max_fee_per_blob_gas = U256::from(max_fee_per_blob_gas);
                    warn!(
                        "max_fee_per_blob_gas exceeds the allowed limit, adjusting it to {max_fee_per_blob_gas}"
                    );
                }
            }
        }
        let tx_hash = send_wrapped_transaction(client, wrapped_tx, signer).await?;

        if number_of_retries > 0 {
            warn!(
                "Resending Transaction after bumping gas, attempts [{number_of_retries}/{}]\nTxHash: {tx_hash:#x}",
                client.max_number_of_retries
            );
        }

        let mut receipt = client.get_transaction_receipt(tx_hash).await?;

        let mut attempt = 1;
        let attempts_to_wait_in_seconds = client
            .backoff_factor
            .pow(number_of_retries as u32)
            .clamp(client.min_retry_delay, client.max_retry_delay);
        while receipt.is_none() {
            if attempt >= (attempts_to_wait_in_seconds / WAIT_TIME_FOR_RECEIPT_SECONDS) {
                // We waited long enough for the receipt but did not find it, bump gas
                // and go to the next one.
                match wrapped_tx {
                    WrappedTransaction::EIP4844(wrapped_eip4844_transaction) => {
                        client.bump_eip4844(wrapped_eip4844_transaction, 30);
                    }
                    WrappedTransaction::EIP1559(eip1559_transaction) => {
                        client.bump_eip1559(eip1559_transaction, 30);
                    }
                    WrappedTransaction::L2(privileged_l2_transaction) => {
                        client.bump_privileged_l2(privileged_l2_transaction, 30);
                    }
                }

                number_of_retries += 1;
                continue 'outer;
            }

            attempt += 1;

            tokio::time::sleep(std::time::Duration::from_secs(
                WAIT_TIME_FOR_RECEIPT_SECONDS,
            ))
            .await;

            receipt = client.get_transaction_receipt(tx_hash).await?;
        }

        return Ok(tx_hash);
    }

    Err(EthClientError::TimeoutError)
}
