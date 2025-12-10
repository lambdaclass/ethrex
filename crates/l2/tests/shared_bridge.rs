#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::too_many_arguments)]

use anyhow::{Context, Result};
use ethrex_common::{Address, H160, U256, types::TxType};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::{
    clients::get_batch_by_block,
    signer::{LocalSigner, Signer},
};
use ethrex_l2_sdk::{
    build_generic_tx, calldata::encode_calldata, compile_contract, create_deploy,
    get_last_verified_batch, git_clone, send_generic_transaction, transfer,
};
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
use std::{fs::File, io::BufRead};
use std::{
    io::BufReader,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};
use tokio::time::sleep;

const L2A_RPC_URL: &str = "http://localhost:1729";
const L2B_RPC_URL: &str = "http://localhost:1730";

const RECEIVER_ADDRESS: &str = "0x000a523148845bee3ee1e9f83df8257a1191c85b";
const SENDER_ADDRESS: &str = "0x000130bade00212be1aa2f4acfe965934635c9cd";
const COMMON_BRIDGE_ADDRESS: &str = "0x000000000000000000000000000000000000FFFF";

const SENDER_PRIVATE_KEY: &str = "029227c59d8967cbfec97cffa4bcfb985852afbd96b7b5da7c9a9a42f92e9166";

const VALUE: u64 = 10000000000000001u64;

const L2_A_CHAIN_ID: u64 = 65536999u64;

const DEST_GAS_LIMIT: u64 = 100000u64;

const SIGNATURE: &str = "sendToL2(uint256,address,uint256,bytes)";

const GAS_PRICE: u64 = 3946771033u64;

// 0x84307998a57635ccc4ed1e5dba1e76344dcdfbe6
const DEFAULT_ON_CHAIN_PROPOSER_ADDRESS: Address = H160([
    0x84, 0x30, 0x79, 0x98, 0xa5, 0x76, 0x35, 0xcc, 0xc4, 0xed, 0x1e, 0x5d, 0xba, 0x1e, 0x76, 0x34,
    0x4d, 0xcd, 0xfb, 0xe6,
]);

fn on_chain_proposer_address() -> Address {
    std::env::var("ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS")
        .map(|address| address.parse().expect("Invalid proposer address"))
        .unwrap_or(DEFAULT_ON_CHAIN_PROPOSER_ADDRESS)
}

pub fn read_env_file_by_config() {
    let env_file_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cmd/.env");
    let Ok(env_file) = File::open(env_file_path) else {
        println!(".env file not found, skipping");
        return;
    };

    let reader = BufReader::new(env_file);

    for line in reader.lines() {
        let line = line.expect("Failed to read line");
        if line.starts_with("#") {
            // Skip comments
            continue;
        };
        match line.split_once('=') {
            Some((key, value)) => {
                if std::env::vars().any(|(k, _)| k == key) {
                    continue;
                }
                unsafe { std::env::set_var(key, value) }
            }
            None => continue,
        };
    }
}

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

    println!("Deploying counter contract on L2A...");
    let counter = compile_and_deploy_counter(l2a_client.clone(), private_key)
        .await
        .expect("Error deploying counter contract");

    println!("Getting initial counter state...");
    let counter_balance = l2a_client
        .get_balance(counter, BlockIdentifier::Tag(BlockTag::Latest))
        .await
        .expect("Error getting counter balance");

    let counter_value = l2a_client
        .call(
            counter,
            encode_calldata("get()", &[]).unwrap().into(),
            Overrides::default(),
        )
        .await
        .unwrap();
    let counter_value_u256 = U256::from_str(&counter_value).unwrap();

    let sender_balance = l2b_client
        .get_balance(sender_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await
        .expect("Error getting balance");

    let value = U256::from(VALUE);
    let to = Address::from_str(COMMON_BRIDGE_ADDRESS).unwrap();
    let data = vec![
        Value::Uint(U256::from(L2_A_CHAIN_ID)),            // chainId
        Value::Address(counter),                           // to
        Value::Uint(U256::from(DEST_GAS_LIMIT)),           // destGasLimit
        Value::Bytes(vec![0xd0, 0x9d, 0xe0, 0x8a].into()), // data
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

    println!("Getting final counter state...");
    let counter_balance_after = l2a_client
        .get_balance(counter, BlockIdentifier::Tag(BlockTag::Latest))
        .await
        .expect("Error getting counter balance");

    let counter_value_after = l2a_client
        .call(
            counter,
            encode_calldata("get()", &[]).unwrap().into(),
            Overrides::default(),
        )
        .await
        .unwrap();
    let counter_value_u256_after = U256::from_str(&counter_value_after).unwrap();

    let sender_balance_after = l2b_client
        .get_balance(sender_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await
        .expect("Error getting balance");

    assert_eq!(
        counter_balance_after,
        counter_balance + value,
        "Counter balance did not increase correctly"
    );

    assert_eq!(
        counter_value_u256_after,
        counter_value_u256 + U256::one(),
        "Counter value did not increase correctly"
    );

    assert!(
        sender_balance_after < sender_balance - value,
        "Sender balance did not decrease correctly"
    );
}

#[tokio::test]
async fn test_forced_inclusion() {
    // The porpuse of this test is to verify that an L2 (L2A) that ignores messages from another L2 (L2B)
    // is unable to advance its lastVerifiedBatch.
    // This test assumes that all the necessary setup has been done to have L2A ignoring messages from L2B
    read_env_file_by_config();
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

    println!("Waiting 5 minutes for message to be expired...");
    sleep(Duration::from_secs(300)).await;

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
        receiver_balance_after, receiver_balance,
        "Receiver balance should not have changed"
    );
    assert!(
        sender_balance_after < sender_balance - value,
        "Sender balance did not decrease correctly"
    );
    println!(
        "Sending a non-privileged transaction on L2A to verify that its batch is never verified..."
    );
    let tx_hash = transfer(
        U256::from(1u64),
        sender_address,
        receiver_address,
        &private_key,
        &l2a_client,
    )
    .await
    .expect("Error sending transfer transaction");
    let receipt = ethrex_l2_sdk::wait_for_transaction_receipt(tx_hash, &l2a_client, 1000)
        .await
        .expect("Error getting receipt for transfer transaction");

    let block_number = receipt.block_info.block_number;
    let mut batch = get_batch_by_block(&l2a_client, BlockIdentifier::Number(block_number))
        .await
        .expect("Failed to get batch by block");
    while batch.is_none() {
        println!("Batch not found yet, waiting 10 seconds...");
        sleep(Duration::from_secs(10)).await;
        batch = get_batch_by_block(&l2a_client, BlockIdentifier::Number(block_number))
            .await
            .expect("Failed to get batch by block");
    }
    println!("Waiting 10 minutes for L2A to try to verify the batch...");
    sleep(Duration::from_secs(600)).await; // Wait for the batch to be verified
    let last_verified_batch = get_last_verified_batch(&l2a_client, on_chain_proposer_address())
        .await
        .expect("Failed to get last verified batch");

    let batch_number = batch.unwrap().batch.number;

    println!(
        "Last verified batch: {}, transaction batch number: {}",
        last_verified_batch, batch_number
    );

    assert!(
        last_verified_batch < batch_number,
        "L2A should not have verified the batch from L2B"
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

async fn compile_and_deploy_counter(
    l2_client: EthClient,
    rich_wallet_private_key: SecretKey,
) -> Result<Address> {
    let contracts_path = Path::new("contracts");

    get_contract_dependencies(contracts_path);
    let remappings = [(
        "@openzeppelin/contracts",
        contracts_path
            .join("lib/openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts/contracts"),
    )];
    compile_contract(
        contracts_path,
        &contracts_path.join("src/example/Counter.sol"),
        false,
        false,
        Some(&remappings),
        &[contracts_path],
    )?;
    let init_code_l2 = hex::decode(String::from_utf8(std::fs::read(
        "contracts/solc_out/Counter.bin",
    )?)?)?;

    let counter = test_deploy(
        &l2_client,
        &init_code_l2,
        &rich_wallet_private_key,
        "test_shared_bridge",
    )
    .await?;

    Ok(counter)
}

fn get_contract_dependencies(contracts_path: &Path) {
    std::fs::create_dir_all(contracts_path.join("lib")).expect("Failed to create contracts/lib");
    git_clone(
        "https://github.com/OpenZeppelin/openzeppelin-contracts-upgradeable.git",
        contracts_path
            .join("lib/openzeppelin-contracts-upgradeable")
            .to_str()
            .expect("Failed to convert path to str"),
        Some("release-v5.4"),
        true,
    )
    .unwrap();
}

/// Test deploying a contract on L2
async fn test_deploy(
    l2_client: &EthClient,
    init_code: &[u8],
    deployer_private_key: &SecretKey,
    test_name: &str,
) -> Result<Address> {
    println!("{test_name}: Deploying contract on L2");

    let deployer: Signer = LocalSigner::new(*deployer_private_key).into();

    let (deploy_tx_hash, contract_address) = create_deploy(
        l2_client,
        &deployer,
        init_code.to_vec().into(),
        Overrides::default(),
    )
    .await?;

    let deploy_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(deploy_tx_hash, l2_client, 50).await?;

    assert!(
        deploy_tx_receipt.receipt.status,
        "{test_name}: Deploy transaction failed"
    );

    Ok(contract_address)
}
