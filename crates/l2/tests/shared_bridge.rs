#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::too_many_arguments)]

use anyhow::{Context, Result};
use ethrex_common::{Address, U256, types::TxType};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use ethrex_l2_sdk::{build_generic_tx, calldata::encode_calldata, send_generic_transaction};
use ethrex_rpc::{
    EthClient,
    clients::Overrides,
    types::{
        block_identifier::{BlockIdentifier, BlockTag},
        receipt::RpcReceipt,
    },
};
use reqwest::Url;
use secp256k1::SecretKey;
use std::{str::FromStr, time::Duration};
use tokio::time::sleep;

const L2A_RPC_URL: &str = "http://localhost:1729";
const L2B_RPC_URL: &str = "http://localhost:1730";

const RECEIVER_ADDRESS: &str = "0xe25583099ba105d9ec0a67f5ae86d90e50036425";
const SENDER_ADDRESS: &str = "0x8943545177806ed17b9f23f0a21ee5948ecaa776";
const COMMON_BRIDGE_ADDRESS: &str = "0x000000000000000000000000000000000000FFFF";

const SENDER_PRIVATE_KEY: &str = "bcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31";

const VALUE: u64 = 10000000000000001u64;

const L2_A_CHAIN_ID: u64 = 65536999u64;

const DEST_GAS_LIMIT: u64 = 100000u64;

const SIGNATURE: &str = "sendToL2(uint256,address,uint256,bytes)";

const GAS_PRICE: u64 = 3946771033u64;

#[tokio::test]
async fn test_shared_bridge() {
    let l2a_client = connect(L2A_RPC_URL).await;
    let l2b_client = connect(L2B_RPC_URL).await;

    let receiver_address = Address::from_str(RECEIVER_ADDRESS).unwrap();
    let sender_address = Address::from_str(SENDER_ADDRESS).unwrap();

    println!("Getting initial balances...");
    let receiver_balance = l2a_client
        .get_balance(receiver_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await
        .expect("Error getting balance");

    let sender_balance = l2b_client
        .get_balance(sender_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await
        .expect("Error getting balance");

    let private_key = SecretKey::from_str(SENDER_PRIVATE_KEY).unwrap();
    let value = U256::from(VALUE);
    let to = Address::from_str(COMMON_BRIDGE_ADDRESS).unwrap();
    let data = vec![
        Value::Uint(U256::from(L2_A_CHAIN_ID)),  // chainId
        Value::Address(receiver_address),        // to
        Value::Uint(U256::from(DEST_GAS_LIMIT)), // destGasLimit
        Value::Bytes(vec![].into()),             // data
    ];
    println!("Sending shared bridge transaction...");
    test_send(
        &l2b_client,
        &private_key,
        value,
        GAS_PRICE,
        to,
        SIGNATURE,
        &data,
        "shared bridge test",
    )
    .await
    .expect("Error sending shared bridge transaction");

    println!("Waiting 3 minutes for message to be processed...");
    sleep(Duration::from_secs(180)).await; // Wait for the message to be processed

    println!("Getting final balances...");
    let receiver_balance_after = l2a_client
        .get_balance(receiver_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await
        .expect("Error getting balance");

    let sender_balance_after = l2b_client
        .get_balance(sender_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await
        .expect("Error getting balance");

    assert_eq!(
        receiver_balance_after,
        receiver_balance + value,
        "Receiver balance did not increase correctly"
    );
    assert!(
        sender_balance_after < sender_balance - value,
        "Sender balance did not decrease correctly"
    );
}

async fn connect(rpc_url: &str) -> EthClient {
    let client = EthClient::new(Url::parse(rpc_url).unwrap()).unwrap();

    let mut retries = 0;
    while retries < 20 {
        match client.get_block_number().await {
            Ok(_) => return client,
            Err(_) => {
                println!("Couldn't get block number. Retries: {retries}");
                retries += 1;
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }

    panic!("Couldn't connect to the RPC server")
}

async fn test_send(
    client: &EthClient,
    private_key: &SecretKey,
    value: U256,
    gas_price: u64,
    to: Address,
    signature: &str,
    data: &[Value],
    test: &str,
) -> Result<RpcReceipt> {
    let signer: Signer = LocalSigner::new(*private_key).into();
    let calldata = encode_calldata(signature, data).unwrap().into();
    let mut tx = build_generic_tx(
        client,
        TxType::EIP1559,
        to,
        signer.address(),
        calldata,
        Overrides {
            value: Some(value),
            max_fee_per_gas: Some(gas_price),
            max_priority_fee_per_gas: Some(gas_price),
            ..Default::default()
        },
    )
    .await
    .with_context(|| format!("Failed to build tx for {test}"))?;
    tx.gas = tx.gas.map(|g| g * 6 / 5); // (+20%) tx reverts in some cases otherwise
    let tx_hash = send_generic_transaction(client, tx, &signer).await.unwrap();
    ethrex_l2_sdk::wait_for_transaction_receipt(tx_hash, client, 1000)
        .await
        .with_context(|| format!("Failed to get receipt for {test}"))
}
