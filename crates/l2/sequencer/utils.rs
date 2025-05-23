use ethrex_common::{Address, H256};
use ethrex_rpc::{
    clients::{eth::WrappedTransaction, EthClientError, Overrides},
    EthClient,
};
use rand::Rng;
use secp256k1::SecretKey;
use std::time::Duration;
use tokio::time::sleep;

pub async fn sleep_random(sleep_amount: u64) {
    let random_noise: u64 = {
        let mut rng = rand::thread_rng();
        rng.gen_range(0..400)
    };

    sleep(Duration::from_millis(sleep_amount + random_noise)).await;
}

pub async fn send_verify_tx(
    encoded_calldata: Vec<u8>,
    eth_client: &EthClient,
    on_chain_proposer_address: Address,
    l1_address: Address,
    l1_private_key: &SecretKey,
) -> Result<H256, EthClientError> {
    let gas_price = eth_client
        .get_gas_price_with_extra(20)
        .await?
        .try_into()
        .map_err(|_| {
            EthClientError::InternalError("Failed to convert gas_price to a u64".to_owned())
        })?;

    let verify_tx = eth_client
        .build_eip1559_transaction(
            on_chain_proposer_address,
            l1_address,
            encoded_calldata.into(),
            Overrides {
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?;

    let mut tx = WrappedTransaction::EIP1559(verify_tx);

    let verify_tx_hash = eth_client
        .send_tx_bump_gas_exponential_backoff(&mut tx, l1_private_key)
        .await?;

    Ok(verify_tx_hash)
}
