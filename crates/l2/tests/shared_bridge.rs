#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::indexing_slicing)]

use anyhow::{Context, Result};
use ethrex_common::{Address, H160, U256, types::TxType};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::{
    clients::get_batch_by_block,
    signer::{LocalSigner, Signer},
};
use ethrex_l2_sdk::{
    COMMON_BRIDGE_L2_ADDRESS, bridge_address, build_generic_tx, calldata::encode_calldata,
    compile_contract, create_deploy, get_last_verified_batch, git_clone, send_generic_transaction,
    transfer, wait_for_l2_deposit_receipt, wait_for_transaction_receipt,
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

const L1_RPC_URL: &str = "http://localhost:8545";
const L2A_RPC_URL: &str = "http://localhost:1729";
const L2B_RPC_URL: &str = "http://localhost:1730";

const RECEIVER_ADDRESS: &str = "0x000a523148845bee3ee1e9f83df8257a1191c85b";
const SENDER_ADDRESS: &str = "0x000130bade00212be1aa2f4acfe965934635c9cd";
const COMMON_BRIDGE_ADDRESS: &str = "0x000000000000000000000000000000000000FFFF";

const SENDER_PRIVATE_KEY: &str = "029227c59d8967cbfec97cffa4bcfb985852afbd96b7b5da7c9a9a42f92e9166";
const SENDER_PRIVATE_KEY_ERC20: &str =
    "e4f7dc8b199fdaac6693c9c412ea68aed9e1584d193e1c3478d30a6f01f26057";

const VALUE: u64 = 10000000000000001u64;

const L2_A_CHAIN_ID: u64 = 65536999u64;
const L2_B_CHAIN_ID: u64 = 1730u64;

const DEST_GAS_LIMIT: u64 = 100000u64;

const SIGNATURE: &str = "sendToL2(uint256,address,uint256,bytes)";

const GAS_PRICE: u64 = 3946771033u64;

const DEPOSIT_VALUE: u64 = 888899999999u64;

// 0xe481f8ed3efe6ff14b58424a1905d72558951167
const DEFAULT_ON_CHAIN_PROPOSER_ADDRESS: Address = H160([
    0xe4, 0x81, 0xf8, 0xed, 0x3e, 0xfe, 0x6f, 0xf1, 0x4b, 0x58, 0x42, 0x4a, 0x19, 0x05, 0xd7, 0x25,
    0x58, 0x95, 0x11, 0x67,
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
async fn test_shared_bridge() -> Result<()> {
    test_counter().await?;

    test_transfer_erc_20().await?;

    Ok(())
}

async fn test_transfer_erc_20() -> Result<()> {
    let l1_client = connect(L1_RPC_URL).await;
    let l2a_client = connect(L2A_RPC_URL).await;
    let l2b_client = connect(L2B_RPC_URL).await;

    // Get the bridge addresses from the Router contract
    let router = router_address()?;
    let l2a_bridge = get_bridge_address_from_router(&l1_client, router, L2_A_CHAIN_ID).await;
    let l2b_bridge = get_bridge_address_from_router(&l1_client, router, L2_B_CHAIN_ID).await;
    println!("test_transfer_erc_20: L2A bridge: {l2a_bridge:?}, L2B bridge: {l2b_bridge:?}");

    let private_key = SecretKey::from_str(SENDER_PRIVATE_KEY_ERC20).unwrap();
    let signer: Signer = LocalSigner::new(private_key).into();
    let sender_address = signer.address();
    println!("test_transfer_erc_20: Sender address: {sender_address:?}");

    let l1_erc20_contract_address = deploy_l1_erc20(&l1_client, &signer, sender_address).await?;
    let fee_token_contract = build_fee_token_bytecode(l1_erc20_contract_address)?;
    let (l2a_erc20_contract_address, _) = deploy_l2_erc20(
        &l2a_client,
        &signer,
        &fee_token_contract,
        sender_address,
        "L2a",
        "l2a",
    )
    .await?;
    let (l2b_erc20_contract_address, l2b_balance) = deploy_l2_erc20(
        &l2b_client,
        &signer,
        &fee_token_contract,
        sender_address,
        "L2b",
        "l2b",
    )
    .await?;

    println!("Approving and depositing ERC20 from L1 to L2a...");
    approve_and_deposit(
        &l1_client,
        &l2a_client,
        &signer,
        l1_erc20_contract_address,
        l2a_erc20_contract_address,
        sender_address,
    )
    .await?;

    let bridge_balance = test_balance_of(&l1_client, l1_erc20_contract_address, l2a_bridge).await;
    assert_eq!(bridge_balance, U256::from(DEPOSIT_VALUE), "invalid deposit");

    // send ERC20 from L2a to L2b
    let transfer_amount = U256::from(999999u64);
    let signature = "transferERC20(uint256,address,uint256,address,address,uint256)";
    let values = [
        Value::Uint(U256::from(L2_B_CHAIN_ID)),
        Value::Address(sender_address),
        Value::Uint(transfer_amount),
        Value::Address(l2a_erc20_contract_address),
        Value::Address(l2b_erc20_contract_address),
        Value::Uint(U256::from(210000)),
    ];
    test_send(
        &l2a_client,
        &private_key,
        U256::zero(),
        GAS_PRICE,
        COMMON_BRIDGE_L2_ADDRESS,
        signature,
        &values,
        "erc20_shared_bridge",
    )
    .await?;
    sleep(Duration::from_secs(180)).await; // Wait for the message to be processed

    let l2b_new_balance =
        test_balance_of(&l2b_client, l2b_erc20_contract_address, sender_address).await;
    assert_eq!(
        l2b_new_balance,
        l2b_balance + transfer_amount,
        "test_transfer_erc_20: Invalid deposit"
    );

    // Verify L1 bridge deposits accounting was updated correctly.
    // Each L2 has its own bridge on L1, so we use the bridge addresses queried from the Router.
    let l1_deposits_l2a_after = get_bridge_deposits(
        &l1_client,
        l2a_bridge,
        l1_erc20_contract_address,
        l2a_erc20_contract_address,
    )
    .await;
    let l1_deposits_l2b_after = get_bridge_deposits(
        &l1_client,
        l2b_bridge,
        l1_erc20_contract_address,
        l2b_erc20_contract_address,
    )
    .await;

    assert_eq!(
        l1_deposits_l2a_after,
        U256::from(DEPOSIT_VALUE) - transfer_amount,
        "test_transfer_erc_20: L1 deposits for L2a should decrease after L2->L2 transfer"
    );
    assert_eq!(
        l1_deposits_l2b_after, transfer_amount,
        "test_transfer_erc_20: L1 deposits for L2b should increase after L2->L2 transfer"
    );

    Ok(())
}

async fn deploy_l1_erc20(
    l1_client: &EthClient,
    signer: &Signer,
    sender_address: Address,
) -> Result<Address> {
    let init_code_bytes = std::fs::read("../../fixtures/contracts/ERC20/ERC20.bin/TestToken.bin")
        .context("failed to read L1 ERC20 bytecode file")?;
    let init_code_l1 =
        hex::decode(init_code_bytes).context("failed to decode L1 ERC20 bytecode")?;

    let (tx_hash, l1_erc20_contract_address) =
        create_deploy(l1_client, signer, init_code_l1.into(), Overrides::default()).await?;
    wait_for_transaction_receipt(tx_hash, l1_client, 100).await?;
    println!("Deployed L1 ERC20 at {l1_erc20_contract_address:?} in hash {tx_hash:?}");
    let l1_balance = test_balance_of(l1_client, l1_erc20_contract_address, sender_address).await;
    assert_eq!(
        l1_balance,
        // The fee token is a mintable ERC20 that mints 1_000_000 * (10 ** 18) tokens to the deployer
        // This is the value in hexadecimal
        U256::from_str("D3C21BCECCEDA1000000")?,
        "l1 invalid deploy"
    );

    Ok(l1_erc20_contract_address)
}

fn build_fee_token_bytecode(l1_erc20_contract_address: Address) -> Result<Vec<u8>> {
    let contracts_path = Path::new("contracts");
    get_contract_dependencies(contracts_path);

    let fee_token_path = Path::new("../../crates/l2/contracts/src/example");
    let interfaces_path = Path::new("../../crates/l2/contracts/src/l2");
    let remappings = [(
        "@openzeppelin/contracts",
        contracts_path
            .join("lib/openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts/contracts"),
    )];
    let allow_paths = [fee_token_path, interfaces_path, contracts_path];

    compile_contract(
        fee_token_path,
        &fee_token_path.join("FeeToken.sol"),
        false,
        false,
        Some(&remappings),
        &allow_paths,
        None,
    )?;

    let mut fee_token_contract =
        hex::decode(std::fs::read(fee_token_path.join("solc_out/FeeToken.bin"))?)?;
    fee_token_contract.extend_from_slice(&[0u8; 12]);
    fee_token_contract.extend_from_slice(&l1_erc20_contract_address.to_fixed_bytes());

    Ok(fee_token_contract)
}

async fn deploy_l2_erc20(
    client: &EthClient,
    signer: &Signer,
    fee_token_contract: &[u8],
    sender_address: Address,
    display_label: &str,
    assert_label: &str,
) -> Result<(Address, U256)> {
    let (tx_hash, l2_erc20_contract_address) = create_deploy(
        client,
        signer,
        fee_token_contract.to_vec().into(),
        Overrides::default(),
    )
    .await?;
    wait_for_transaction_receipt(tx_hash, client, 100).await?;
    println!("Deployed {display_label} ERC20 at {l2_erc20_contract_address:?} in hash {tx_hash:?}");
    let balance = test_balance_of(client, l2_erc20_contract_address, sender_address).await;
    assert_eq!(
        balance,
        U256::from_str("D3C21BCECCEDA1000000")?,
        "{assert_label} invalid deploy"
    );

    Ok((l2_erc20_contract_address, balance))
}

async fn approve_and_deposit(
    l1_client: &EthClient,
    l2_client: &EthClient,
    signer: &Signer,
    l1_erc20_contract_address: Address,
    l2_erc20_contract_address: Address,
    sender_address: Address,
) -> Result<()> {
    let bridge_address = bridge_address()?;
    let approve_calldata = encode_calldata(
        "approve(address,uint256)",
        &[
            Value::Address(bridge_address),
            Value::Uint(U256::from(9999999999999999u64)),
        ],
    )?;
    let approve_tx = build_generic_tx(
        l1_client,
        TxType::EIP1559,
        l1_erc20_contract_address,
        sender_address,
        approve_calldata.into(),
        Overrides::default(),
    )
    .await?;
    println!("Approving ERC20 transfer to bridge at {bridge_address:?}...");
    let approve_hash = send_generic_transaction(l1_client, approve_tx, signer).await?;
    wait_for_transaction_receipt(approve_hash, l1_client, 100).await?;

    let deposit_calldata = encode_calldata(
        "depositERC20(address,address,address,uint256)",
        &[
            Value::Address(l1_erc20_contract_address),
            Value::Address(l2_erc20_contract_address),
            Value::Address(sender_address),
            Value::Uint(U256::from(DEPOSIT_VALUE)),
        ],
    )?;
    let deposit_tx = build_generic_tx(
        l1_client,
        TxType::EIP1559,
        bridge_address,
        sender_address,
        deposit_calldata.into(),
        Overrides::default(),
    )
    .await?;
    println!("Depositing ERC20 to L2 at {l2_erc20_contract_address:?}...");
    let deposit_hash = send_generic_transaction(l1_client, deposit_tx, signer).await?;
    let deposit_receipt = wait_for_transaction_receipt(deposit_hash, l1_client, 100).await?;
    wait_for_l2_deposit_receipt(&deposit_receipt, l1_client, l2_client).await?;

    Ok(())
}

async fn test_counter() -> Result<()> {
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
    Ok(())
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

    println!("Initial balances: receiver: {receiver_balance}, sender: {sender_balance}");

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
    let on_chain_proposer_address = on_chain_proposer_address();
    println!("Using onChainProposer address: {on_chain_proposer_address:?}");
    let last_verified_batch = get_last_verified_batch(&l2a_client, on_chain_proposer_address)
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
        None,
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

async fn test_balance_of(client: &EthClient, token: Address, user: Address) -> U256 {
    let res = client
        .call(
            token,
            encode_calldata("balanceOf(address)", &[Value::Address(user)])
                .unwrap()
                .into(),
            Default::default(),
        )
        .await
        .unwrap();
    U256::from_str_radix(res.trim_start_matches("0x"), 16).unwrap()
}

/// Reads the `deposits(address,address)` mapping from the L1 CommonBridge contract.
async fn get_bridge_deposits(
    client: &EthClient,
    bridge: Address,
    token_l1: Address,
    token_l2: Address,
) -> U256 {
    let res = client
        .call(
            bridge,
            encode_calldata(
                "deposits(address,address)",
                &[Value::Address(token_l1), Value::Address(token_l2)],
            )
            .unwrap()
            .into(),
            Default::default(),
        )
        .await
        .unwrap();
    U256::from_str_radix(res.trim_start_matches("0x"), 16).unwrap()
}

/// Gets the Router contract address from environment variable.
fn router_address() -> Result<Address> {
    std::env::var("ETHREX_SHARED_BRIDGE_ROUTER_ADDRESS")
        .context("ETHREX_SHARED_BRIDGE_ROUTER_ADDRESS not set")?
        .parse()
        .context("Invalid router address")
}

/// Queries the Router contract's `bridges(uint256)` mapping to get the bridge address for a chain.
async fn get_bridge_address_from_router(
    client: &EthClient,
    router: Address,
    chain_id: u64,
) -> Address {
    let res = client
        .call(
            router,
            encode_calldata("bridges(uint256)", &[Value::Uint(U256::from(chain_id))])
                .unwrap()
                .into(),
            Default::default(),
        )
        .await
        .unwrap();
    let bytes = hex::decode(res.trim_start_matches("0x")).unwrap();
    Address::from_slice(&bytes[12..32])
}
