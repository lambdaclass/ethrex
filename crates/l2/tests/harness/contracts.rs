#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use std::path::Path;

use bytes::Bytes;
use ethrex_common::{Address, U256};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::clients::deploy;
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_l2_sdk::{
    COMMON_BRIDGE_L2_ADDRESS, L1ToL2TransactionData, bridge_address, compile_contract,
    get_erc1967_slot, wait_for_transaction_receipt,
};
use ethrex_rpc::EthClient;
use ethrex_rpc::clients::Overrides;
use ethrex_rpc::types::block_identifier::{BlockIdentifier, BlockTag};
use keccak_hash::keccak;
use secp256k1::SecretKey;

use crate::harness::eth::test_send;
use crate::harness::utils::{
    fees_vault, get_contract_dependencies, get_fees_details_l2, l1_client, l2_client, rich_pk_1,
    wait_for_l2_ptx_receipt,
};

pub async fn test_deploy_l1(
    client: &EthClient,
    init_code: &[u8],
    private_key: &SecretKey,
) -> Result<Address, Box<dyn std::error::Error>> {
    println!("Deploying contract on L1");

    let deployer_signer: Signer = LocalSigner::new(*private_key).into();

    let (deploy_tx_hash, contract_address) = deploy(
        client,
        &deployer_signer,
        init_code.to_vec().into(),
        Overrides::default(),
    )
    .await?;

    ethrex_l2_sdk::wait_for_transaction_receipt(deploy_tx_hash, client, 5).await?;

    Ok(contract_address)
}

pub async fn test_deploy(
    l2_client: &EthClient,
    init_code: &[u8],
    deployer_private_key: &SecretKey,
) -> Result<Address, Box<dyn std::error::Error>> {
    println!("Deploying contract on L2");

    let deployer: Signer = LocalSigner::new(*deployer_private_key).into();

    let deployer_balance_before_deploy = l2_client
        .get_balance(deployer.address(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    let fee_vault_balance_before_deploy = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    let (deploy_tx_hash, contract_address) = deploy(
        l2_client,
        &deployer,
        init_code.to_vec().into(),
        Overrides::default(),
    )
    .await?;

    let deploy_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(deploy_tx_hash, l2_client, 5).await?;

    let deploy_fees = get_fees_details_l2(deploy_tx_receipt, l2_client).await;

    let deployer_balance_after_deploy = l2_client
        .get_balance(deployer.address(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert_eq!(
        deployer_balance_after_deploy,
        deployer_balance_before_deploy - deploy_fees.total_fees,
        "Deployer L2 balance didn't decrease as expected after deploy"
    );

    let fee_vault_balance_after_deploy = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert_eq!(
        fee_vault_balance_after_deploy,
        fee_vault_balance_before_deploy + deploy_fees.recoverable_fees,
        "Fee vault balance didn't increase as expected after deploy"
    );

    let deployed_contract_balance = l2_client
        .get_balance(contract_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert!(
        deployed_contract_balance.is_zero(),
        "Deployed contract balance should be zero after deploy"
    );

    Ok(contract_address)
}

pub async fn test_upgrade() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing upgrade");
    let l1_client = l1_client();
    let l2_client = l2_client();
    let private_key = rich_pk_1();

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
        &[contracts_path],
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

    let _ =
        wait_for_l2_ptx_receipt(tx_receipt.block_info.block_number, &l1_client, &l2_client).await?;
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
pub async fn test_privileged_tx_with_contract_call() -> Result<(), Box<dyn std::error::Error>> {
    let l1_client = l1_client();
    let l2_client = l2_client();
    let rich_wallet_private_key = rich_pk_1();
    println!("ptx_with_contract_call: Deploying contract on L2");

    let init_code = hex::decode(std::fs::read(
        "../../fixtures/contracts/payable/Payable.bin",
    )?)?;
    let deployed_contract_address =
        test_deploy(&l2_client, &init_code, &rich_wallet_private_key).await?;

    let number_to_emit = U256::from(424242);
    let calldata_to_contract: Bytes = encode_calldata(
        "functionThatEmitsEvent(uint256)",
        &[Value::Uint(number_to_emit)],
    )?
    .into();

    // We need to get the block number before the deposit to search for logs later.
    let first_block = l2_client.get_block_number().await?;

    println!("ptx_with_contract_call: Calling contract with deposit");

    test_call_to_contract_with_transfer(
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

    Ok(())
}

pub async fn test_privileged_tx_with_contract_call_revert() -> Result<(), Box<dyn std::error::Error>>
{
    let l1_client = l1_client();
    let l2_client = l2_client();
    let rich_wallet_private_key = rich_pk_1();
    let init_code = hex::decode(std::fs::read(
        "../../fixtures/contracts/payable/Payable.bin",
    )?)?;
    println!("ptx_with_contract_call_revert: Deploying contract on L2");

    let deployed_contract_address =
        test_deploy(&l2_client, &init_code, &rich_wallet_private_key).await?;

    let calldata_to_contract: Bytes = encode_calldata("functionThatReverts()", &[])?.into();

    println!("ptx_with_contract_call_revert: Calling contract with deposit");

    test_call_to_contract_with_transfer(
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
pub async fn test_call_to_contract_with_transfer(
    l1_client: &EthClient,
    l2_client: &EthClient,
    deployed_contract_address: Address,
    calldata_to_contract: Bytes,
    caller_private_key: &SecretKey,
    value: U256,
    should_revert: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let caller_address = ethrex_l2_sdk::get_address_from_secret_key(caller_private_key)
        .expect("Failed to get address");

    println!("Checking balances before call");

    let caller_l1_balance_before_call = l1_client
        .get_balance(caller_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    let deployed_contract_balance_before_call = l2_client
        .get_balance(
            deployed_contract_address,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await?;

    let fee_vault_balance_before_call = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    println!("Calling contract on L2 with transfer");

    let l1_to_l2_tx_hash = ethrex_l2_sdk::send_l1_to_l2_tx(
        caller_address,
        Some(0),
        None,
        L1ToL2TransactionData::new(
            deployed_contract_address,
            21000 * 5,
            value,
            calldata_to_contract.clone(),
        ),
        caller_private_key,
        bridge_address()?,
        l1_client,
    )
    .await?;

    println!("Waiting for L1 to L2 transaction receipt on L1");

    let l1_to_l2_tx_receipt = wait_for_transaction_receipt(l1_to_l2_tx_hash, l1_client, 5).await?;

    assert!(l1_to_l2_tx_receipt.receipt.status);

    println!("Waiting for L1 to L2 transaction receipt on L2");

    let _ = wait_for_l2_ptx_receipt(
        l1_to_l2_tx_receipt.block_info.block_number,
        l1_client,
        l2_client,
    )
    .await?;

    println!("Checking balances after call");

    let caller_l1_balance_after_call = l1_client
        .get_balance(caller_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert_eq!(
        caller_l1_balance_after_call,
        caller_l1_balance_before_call
            - l1_to_l2_tx_receipt.tx_info.gas_used
                * l1_to_l2_tx_receipt.tx_info.effective_gas_price,
        "Caller L1 balance didn't decrease as expected after call"
    );

    let fee_vault_balance_after_call = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert_eq!(
        fee_vault_balance_after_call, fee_vault_balance_before_call,
        "Fee vault balance increased unexpectedly after call"
    );

    let deployed_contract_balance_after_call = l2_client
        .get_balance(
            deployed_contract_address,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await?;

    let value = if should_revert { U256::zero() } else { value };

    assert_eq!(
        deployed_contract_balance_before_call + value,
        deployed_contract_balance_after_call,
        "Deployed contract final balance was not expected"
    );

    Ok(())
}
