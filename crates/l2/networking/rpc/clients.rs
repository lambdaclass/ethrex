use crate::signer::{Signable, Signer};
use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{EIP1559Transaction, GenericTransaction, TxKind, TxType, WrappedEIP4844Transaction},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::{
    clients::{EthClientError, Overrides, eth::EthClient},
    types::block_identifier::{BlockIdentifier, BlockTag},
};
use keccak_hash::keccak;
use std::ops::Div;
use tracing::warn;

const WAIT_TIME_FOR_RECEIPT_SECONDS: u64 = 2;

pub async fn send_generic_transaction(
    client: &EthClient,
    generic_tx: GenericTransaction,
    signer: &Signer,
) -> Result<H256, EthClientError> {
    let mut encoded_tx = vec![generic_tx.r#type.into()];
    match generic_tx.r#type {
        TxType::EIP1559 => {
            let tx: EIP1559Transaction = generic_tx.try_into()?;
            let signed_tx = tx
                .sign(signer)
                .await
                .map_err(|err| EthClientError::Custom(err.to_string()))?;

            signed_tx.encode(&mut encoded_tx);
        }
        TxType::EIP4844 => {
            let mut tx: WrappedEIP4844Transaction = generic_tx.try_into()?;
            tx.tx
                .sign_inplace(signer)
                .await
                .map_err(|err| EthClientError::Custom(err.to_string()))?;

            tx.encode(&mut encoded_tx);
        }
        _ => {
            return Err(EthClientError::Custom(
                "Unsupported transaction type".to_string(),
            ));
        }
    };
    client.send_raw_transaction(encoded_tx.as_slice()).await
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
        .build_generic_tx(
            TxType::EIP1559,
            Address::zero(),
            deployer.address(),
            init_code,
            deploy_overrides,
        )
        .await?;
    let deploy_tx_hash = send_generic_transaction(client, deploy_tx, deployer).await?;

    let nonce = client
        .get_nonce(deployer.address(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;
    let mut encode = vec![];
    (deployer.address(), nonce).encode(&mut encode);

    //Taking the last 20bytes so it matches an H160 == Address length
    let deployed_address = Address::from_slice(keccak(encode).as_fixed_bytes().get(12..).ok_or(
        EthClientError::Custom("Failed to get deployed_address".to_owned()),
    )?);

    client
        .wait_for_transaction_receipt(deploy_tx_hash, 1000)
        .await?;

    Ok((deploy_tx_hash, deployed_address))
}

pub async fn send_tx_bump_gas_exponential_backoff(
    client: &EthClient,
    mut tx: GenericTransaction,
    signer: &Signer,
) -> Result<H256, EthClientError> {
    let mut number_of_retries = 0;

    'outer: while number_of_retries < client.max_number_of_retries {
        if let Some(max_fee_per_gas) = client.maximum_allowed_max_fee_per_gas {
            let (Some(tx_max_fee), Some(tx_max_priority_fee)) =
                (&mut tx.max_fee_per_gas, &mut tx.max_priority_fee_per_gas)
            else {
                return Err(EthClientError::Custom(
                    "Invalid transaction: max_fee_per_gas or max_priority_fee_per_gas is missing"
                        .to_string(),
                ));
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
        if let Some(tx_max_fee_per_blob_gas) = &mut tx.max_fee_per_blob_gas {
            if let Some(max_fee_per_blob_gas) = client.maximum_allowed_max_fee_per_blob_gas {
                if *tx_max_fee_per_blob_gas > U256::from(max_fee_per_blob_gas) {
                    *tx_max_fee_per_blob_gas = U256::from(max_fee_per_blob_gas);
                    warn!(
                        "max_fee_per_blob_gas exceeds the allowed limit, adjusting it to {max_fee_per_blob_gas}"
                    );
                }
            }
        }
        let Ok(tx_hash) = send_generic_transaction(client, tx.clone(), signer).await else {
            bump_gas_generic_tx(&mut tx, 30);
            number_of_retries += 1;
            continue;
        };

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
                bump_gas_generic_tx(&mut tx, 30);

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

fn bump_gas_generic_tx(tx: &mut GenericTransaction, bump_percentage: u64) {
    if let (Some(max_fee_per_gas), Some(max_priority_fee_per_gas)) =
        (&mut tx.max_fee_per_gas, &mut tx.max_priority_fee_per_gas)
    {
        *max_fee_per_gas = (*max_fee_per_gas * (100 + bump_percentage)) / 100;
        *max_priority_fee_per_gas = (*max_priority_fee_per_gas * (100 + bump_percentage)) / 100;
    }
    if let Some(max_fee_per_blob_gas) = &mut tx.max_fee_per_blob_gas {
        let factor = 1 + (bump_percentage / 100) * 10;
        *max_fee_per_blob_gas = max_fee_per_blob_gas
            .saturating_mul(U256::from(factor))
            .div(10);
    }
}
