use std::path::Path;

use bytes::Bytes;
use ethrex_common::U256;
use ethrex_l2_common::calldata::Value;
use ethrex_l2_sdk::{bridge_address, compile_contract, get_erc1967_slot, COMMON_BRIDGE_L2_ADDRESS};
use ethrex_rpc::types::block_identifier::{BlockIdentifier, BlockTag};
use ethrex_rpc::clients::eth::EthClient;
use secp256k1::SecretKey;
use ethrex_l2_sdk::calldata::encode_calldata;
use keccak_hash::keccak;
use ethrex_common::Address;
use ethrex_l2_sdk::{wait_for_transaction_receipt, L1ToL2TransactionData, send_l1_to_l2_tx};
use crate::common::{fees_vault, test_call_to_contract_with_deposit};

use crate::common::{accounts::get_rich_account, get_contract_dependencies, l1_client, l2_client, test_deploy, test_send, wait_for_l2_deposit_receipt};

mod common;

#[tokio::test]
async fn test_upgrade() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing upgrade");
    let l1_client = l1_client();
    let l2_client = l2_client();
    let private_key = get_rich_account().await;

    println!("test upgrade: Downloading openzeppelin contracts");

    let contracts_path = Path::new("contracts");
    get_contract_dependencies(contracts_path);
    let remappings = [(
        "@openzeppelin/contracts",
        contracts_path
            .join("lib/openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts/contracts"),
    )];

    println!("test upgrade: Compiling CommonBridgeL2 contract");
    compile_contract(
        contracts_path,
        Path::new("contracts/src/l2/CommonBridgeL2.sol"),
        false,
        Some(&remappings),
    )?;

    let bridge_code = hex::decode(std::fs::read("contracts/solc_out/CommonBridgeL2.bin")?)?;

    println!("test upgrade: Deploying CommonBridgeL2 contract");
    let deploy_address = test_deploy(&l2_client, &bridge_code, &private_key).await?;

    let impl_slot = get_erc1967_slot("eip1967.proxy.implementation");
    let initial_impl = l2_client
        .get_storage_at(
            COMMON_BRIDGE_L2_ADDRESS,
            impl_slot,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await?;

    println!("test upgrade: Upgrading CommonBridgeL2 contract");
    let tx_receipt = test_send(
        &l1_client,
        &private_key,
        bridge_address()?,
        "upgradeL2Contract(address,address,uint256,bytes)",
        &[
            Value::Address(COMMON_BRIDGE_L2_ADDRESS),
            Value::Address(deploy_address),
            Value::Uint(U256::from(100_000)),
            Value::Bytes(Bytes::new()),
        ],
    )
    .await;

    assert!(tx_receipt.receipt.status, "Upgrade transaction failed");

    let _ = wait_for_l2_deposit_receipt(tx_receipt.block_info.block_number, &l1_client, &l2_client)
        .await?;
    let final_impl = l2_client
        .get_storage_at(
            COMMON_BRIDGE_L2_ADDRESS,
            impl_slot,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await?;
    println!("test upgrade: upgraded {initial_impl:#x} -> {final_impl:#x}");
    assert_ne!(initial_impl, final_impl);
    Ok(())
}

/// In this test we deploy a contract on L2 and call it from L1 using the CommonBridge contract.
/// We call the contract by making a deposit from L1 to L2 with the recipient being the rich account.
/// The deposit will trigger the call to the contract.
#[tokio::test]
async fn test_privileged_tx_with_contract_call() -> Result<(), Box<dyn std::error::Error>> {
    let l1_client = l1_client();
    let l2_client = l2_client();
    let rich_wallet_private_key = get_rich_account().await;
    println!("ptx_with_contract_call: Deploying contract on L2");

    let init_code = hex::decode(std::fs::read(
        "../../fixtures/contracts/payable/Payable.bin",
    )?)?;
    let deployed_contract_address =
        test_deploy(&l2_client, &init_code, &rich_wallet_private_key).await?;

    let payable_initial_balance = l2_client
        .get_balance(
            deployed_contract_address,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await?;

    let number_to_emit = U256::from(424242);
    let calldata_to_contract: Bytes = encode_calldata(
        "functionThatEmitsEvent(uint256)",
        &[Value::Uint(number_to_emit)],
    )?
    .into();

    // We need to get the block number before the deposit to search for logs later.
    let first_block = l2_client.get_block_number().await?;

    println!("ptx_with_contract_call: Calling contract with deposit");

    test_call_to_contract_with_deposit(
        &l1_client,
        &l2_client,
        deployed_contract_address,
        calldata_to_contract,
        &rich_wallet_private_key,
        U256::from(1),
        false,
    )
    .await?;

    println!("ptx_with_contract_call: Waiting for event to be emitted");

    let mut block_number = first_block;

    let topic = keccak(b"Number(uint256)");

    while l2_client
        .get_logs(
            first_block,
            block_number,
            deployed_contract_address,
            vec![topic],
        )
        .await
        .is_ok_and(|logs| logs.is_empty())
    {
        println!("ptx_with_contract_call: Waiting for the event to be built");
        block_number += U256::one();
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    println!("ptx_with_contract_call: Event found in block {block_number}");

    let logs = l2_client
        .get_logs(
            first_block,
            block_number,
            deployed_contract_address,
            vec![topic],
        )
        .await?;

    let number_emitted = U256::from_big_endian(
        &logs
            .first()
            .unwrap()
            .log
            .topics
            .get(1)
            .unwrap()
            .to_fixed_bytes(),
    );

    assert_eq!(
        number_emitted, number_to_emit,
        "Event emitted with wrong value. Expected 424242, got {number_emitted}"
    );

    let payable_final_balance = l2_client
        .get_balance(
            deployed_contract_address,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await?;

    assert_eq!(payable_initial_balance + 1, payable_final_balance);

    Ok(())
}

/// Test the deployment of a contract on L2 and call it from L1 using the CommonBridge contract.
/// The call to the contract should revert but the deposit should be successful.
#[tokio::test]
async fn test_privileged_tx_with_contract_call_revert() -> Result<(), Box<dyn std::error::Error>> {
    let l1_client = l1_client();
    let l2_client = l2_client();
    let rich_wallet_private_key = get_rich_account().await;
    let init_code = hex::decode(std::fs::read(
        "../../fixtures/contracts/payable/Payable.bin",
    )?)?;
    let deployed_contract_address =
        test_deploy(&l2_client, &init_code, &rich_wallet_private_key).await?;

    let payable_initial_balance = l2_client
        .get_balance(
            deployed_contract_address,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await?;

    println!("ptx_with_contract_call_revert: Deploying contract on L2");

    let deployed_contract_address =
        test_deploy(&l2_client, &init_code, &rich_wallet_private_key).await?;

    let calldata_to_contract: Bytes = encode_calldata("functionThatReverts()", &[])?.into();

    println!("ptx_with_contract_call_revert: Calling contract with deposit");

    test_call_to_contract_with_deposit(
        &l1_client,
        &l2_client,
        deployed_contract_address,
        calldata_to_contract,
        &rich_wallet_private_key,
        U256::from(1),
        true,
    )
    .await?;

    Ok(())
}
