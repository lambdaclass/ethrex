#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
use bytes::Bytes;
use ethereum_types::{Address, U256};
use ethrex_common::types::BlockNumber;
use ethrex_common::H160;
use ethrex_l2_sdk::calldata::{self, Value};
use ethrex_l2_sdk::client::eth::BlockByNumber;
use ethrex_l2_sdk::client::{EthClient, Overrides};
use ethrex_l2_sdk::l1_to_l2_tx_data::L1ToL2TransactionData;
use ethrex_l2_sdk::wait_for_transaction_receipt;
use ethrex_rpc::types::receipt::RpcReceipt;
use keccak_hash::{keccak, H256};
use secp256k1::SecretKey;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::{ops::Mul, str::FromStr, time::Duration};

const DEFAULT_ETH_URL: &str = "http://localhost:8545";
const DEFAULT_PROPOSER_URL: &str = "http://localhost:1729";
// 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e
const DEFAULT_L1_RICH_WALLET_PRIVATE_KEY: H256 = H256([
    0x94, 0x1e, 0x10, 0x33, 0x20, 0x61, 0x5d, 0x39, 0x4a, 0x55, 0x70, 0x8b, 0xe1, 0x3e, 0x45, 0x99,
    0x4c, 0x7d, 0x93, 0xb9, 0x32, 0xb0, 0x64, 0xdb, 0xcb, 0x2b, 0x51, 0x1f, 0xe3, 0x25, 0x4e, 0x2e,
]);
// 0x8ccf74999c496e4d27a2b02941673f41dd0dab2a
const DEFAULT_BRIDGE_ADDRESS: Address = H160([
    0x8c, 0xcf, 0x74, 0x99, 0x9c, 0x49, 0x6e, 0x4d, 0x27, 0xa2, 0xb0, 0x29, 0x41, 0x67, 0x3f, 0x41,
    0xdd, 0x0d, 0xab, 0x2a,
]);
// 0x0007a881CD95B1484fca47615B64803dad620C8d
const DEFAULT_PROPOSER_COINBASE_ADDRESS: Address = H160([
    0x00, 0x07, 0xa8, 0x81, 0xcd, 0x95, 0xb1, 0x48, 0x4f, 0xca, 0x47, 0x61, 0x5b, 0x64, 0x80, 0x3d,
    0xad, 0x62, 0x0c, 0x8d,
]);

const L2_GAS_COST_MAX_DELTA: U256 = U256([100_000_000_000_000, 0, 0, 0]);

/// Test the full flow of depositing, transferring, and withdrawing funds
/// from L1 to L2 and back.
///
/// 1. Check balances on L1 and L2
/// 2. Deposit from L1 to L2
/// 3. Check deposit receipt in L2
/// 4. Check balances on L1 and L2
/// 5. Transfer funds on L2
/// 6. Check balances on L2
/// 7. Withdraw funds from L2 to L1
/// 8. Check balances on L1 and L2
/// 9. Claim funds on L1
/// 10. Check balances on L1 and L2
#[tokio::test]
async fn l2_integration_test() -> Result<(), Box<dyn std::error::Error>> {
    read_env_file_by_config();

    let eth_client = eth_client();
    let proposer_client = proposer_client();
    let rich_wallet_private_key = l1_rich_wallet_private_key();
    let bridge_address = common_bridge_address();

    test_deposit(
        &rich_wallet_private_key,
        bridge_address,
        &eth_client,
        &proposer_client,
    )
    .await?;

    test_transfer(&rich_wallet_private_key, &proposer_client).await?;

    test_n_withdraws(&rich_wallet_private_key, &eth_client, &proposer_client, 5).await?;

    println!("l2_integration_test is done");
    Ok(())
}

/// In this test we deploy a contract on L2 and call it from L1 using the CommonBridge contract.
/// We call the contract by making a deposit from L1 to L2 with the recipient being the rich account.
/// The deposit will trigger the call to the contract.
#[tokio::test]
async fn l2_deposit_with_contract_call() -> Result<(), Box<dyn std::error::Error>> {
    read_env_file_by_config();

    let eth_client = eth_client();
    let proposer_client = proposer_client();
    let rich_wallet_private_key = l1_rich_wallet_private_key();
    let bridge_address = common_bridge_address();

    test_deposit(
        &rich_wallet_private_key,
        bridge_address,
        &eth_client,
        &proposer_client,
    )
    .await?;

    // pragma solidity ^0.8.27;
    // contract Test {
    //     event NumberSet(uint256 indexed number);
    //     function emitNumber(uint256 _number) public {
    //         emit NumberSet(_number);
    //     }
    // }
    let init_code = hex::decode("6080604052348015600e575f5ffd5b506101008061001c5f395ff3fe6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063f15d140b14602a575b5f5ffd5b60406004803603810190603c919060a4565b6042565b005b807f9ec8254969d1974eac8c74afb0c03595b4ffe0a1d7ad8a7f82ed31b9c854259160405160405180910390a250565b5f5ffd5b5f819050919050565b6086816076565b8114608f575f5ffd5b50565b5f81359050609e81607f565b92915050565b5f6020828403121560b65760b56072565b5b5f60c1848285016092565b9150509291505056fea26469706673582212206f6d360696127c56e2d2a456f3db4a61e30eae0ea9b3af3c900c81ea062e8fe464736f6c634300081c0033")?;

    let deployed_contract_address =
        test_deploy(&init_code, &rich_wallet_private_key, &proposer_client).await?;

    let number_to_emit = U256::from(424242);
    let calldata_to_contract: Bytes =
        calldata::encode_calldata("emitNumber(uint256)", &[Value::Uint(number_to_emit)])?.into();

    // We need to get the block number before the deposit to search for logs later.
    let first_block = proposer_client.get_block_number().await?;

    test_call_to_contract_with_deposit(
        deployed_contract_address,
        calldata_to_contract,
        &rich_wallet_private_key,
        &proposer_client,
        &eth_client,
    )
    .await?;

    println!("Waiting for event to be emitted");

    let mut block_number = first_block;

    let topic = keccak(b"NumberSet(uint256)");

    while proposer_client
        .get_logs(first_block, block_number, deployed_contract_address, topic)
        .await
        .is_ok_and(|logs| logs.is_empty())
    {
        println!("Waiting for the event to be built");
        block_number += U256::one();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    let logs = proposer_client
        .get_logs(first_block, block_number, deployed_contract_address, topic)
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

/// Test the deployment of a contract on L2 and call it from L1 using the CommonBridge contract.
/// The call to the contract should revert but the deposit should be successful.
#[tokio::test]
async fn l2_deposit_with_contract_call_revert() -> Result<(), Box<dyn std::error::Error>> {
    read_env_file_by_config();

    let eth_client = eth_client();
    let proposer_client = proposer_client();
    let rich_wallet_private_key = l1_rich_wallet_private_key();
    let bridge_address = common_bridge_address();

    test_deposit(
        &rich_wallet_private_key,
        bridge_address,
        &eth_client,
        &proposer_client,
    )
    .await?;

    // pragma solidity ^0.8.27;
    // contract RevertTest {
    //     function revert_call() public {
    //         revert("Reverted");
    //     }
    // }
    let init_code = hex::decode("6080604052348015600e575f5ffd5b506101138061001c5f395ff3fe6080604052348015600e575f5ffd5b50600436106026575f3560e01c806311ebce9114602a575b5f5ffd5b60306032565b005b6040517f08c379a000000000000000000000000000000000000000000000000000000000815260040160629060c1565b60405180910390fd5b5f82825260208201905092915050565b7f52657665727465640000000000000000000000000000000000000000000000005f82015250565b5f60ad600883606b565b915060b682607b565b602082019050919050565b5f6020820190508181035f83015260d68160a3565b905091905056fea2646970667358221220903f571921ce472f979989f9135b8637314b68e080fd70d0da6ede87ad8b5bd564736f6c634300081c0033")?;

    let deployed_contract_address =
        test_deploy(&init_code, &rich_wallet_private_key, &proposer_client).await?;

    let calldata_to_contract: Bytes = calldata::encode_calldata("revert_call()", &[])?.into();

    test_call_to_contract_with_deposit(
        deployed_contract_address,
        calldata_to_contract,
        &rich_wallet_private_key,
        &proposer_client,
        &eth_client,
    )
    .await?;

    Ok(())
}

async fn test_deposit(
    depositor_private_key: &SecretKey,
    bridge_address: Address,
    eth_client: &EthClient,
    proposer_client: &EthClient,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Fetching initial balances on L1 and L2");

    let depositor = ethrex_l2_sdk::get_address_from_secret_key(depositor_private_key)?;
    let deposit_recipient_address = Address::random();
    let deposit_value = U256::from(1000000000000000000000u128);

    let depositor_l1_initial_balance = eth_client
        .get_balance(depositor, BlockByNumber::Latest)
        .await?;

    assert!(
        depositor_l1_initial_balance >= deposit_value,
        "L1 depositor doesn't have enough balance to deposit"
    );

    let deposit_recipient_l2_initial_balance = proposer_client
        .get_balance(deposit_recipient_address, BlockByNumber::Latest)
        .await?;

    let bridge_initial_balance = eth_client
        .get_balance(bridge_address, BlockByNumber::Latest)
        .await?;

    let fee_vault_balance_before_deposit = proposer_client
        .get_balance(fees_vault(), BlockByNumber::Latest)
        .await?;

    println!("Depositing funds from L1 to L2");

    let deposit_tx_hash = ethrex_l2_sdk::deposit_through_contract_call(
        deposit_value,
        deposit_recipient_address,
        21000 * 10,
        21000 * 10,
        depositor_private_key,
        bridge_address,
        eth_client,
    )
    .await?;

    println!("Waiting for L1 deposit transaction receipt");

    let deposit_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(deposit_tx_hash, eth_client, 5).await?;

    let depositor_l1_balance_after_deposit = eth_client
        .get_balance(depositor, BlockByNumber::Latest)
        .await?;

    assert_eq!(
        depositor_l1_balance_after_deposit,
        depositor_l1_initial_balance
            - deposit_value
            - deposit_tx_receipt.tx_info.gas_used * deposit_tx_receipt.tx_info.effective_gas_price,
        "Depositor L1 balance didn't decrease as expected after deposit"
    );

    let bridge_balance_after_deposit = eth_client
        .get_balance(bridge_address, BlockByNumber::Latest)
        .await?;

    assert_eq!(
        bridge_balance_after_deposit,
        bridge_initial_balance + deposit_value,
        "Bridge balance didn't increase as expected after deposit"
    );

    println!("Waiting for L2 deposit tx receipt");

    let deposit_tx_receipt = wait_for_l2_deposit_receipt(
        deposit_tx_receipt.block_info.block_number,
        eth_client,
        proposer_client,
    )
    .await?;

    let deposit_recipient_l2_balance_after_deposit = proposer_client
        .get_balance(deposit_recipient_address, BlockByNumber::Latest)
        .await?;

    assert_eq!(
        deposit_recipient_l2_balance_after_deposit,
        deposit_recipient_l2_initial_balance + deposit_value,
        "Deposit recipient L2 balance didn't increase as expected after deposit"
    );

    let deposit_fees = get_fees_details_l2(deposit_tx_receipt, proposer_client).await;

    let fee_vault_balance_after_deposit = proposer_client
        .get_balance(fees_vault(), BlockByNumber::Latest)
        .await?;

    assert_eq!(
        fee_vault_balance_after_deposit,
        fee_vault_balance_before_deposit + deposit_fees.recoverable_fees,
        "Fee vault balance should not change after deposit"
    );

    Ok(())
}

async fn test_transfer(
    transferer_private_key: &SecretKey,
    proposer_client: &EthClient,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Transferring funds on L2");

    let transfer_recipient_address = Address::random();
    let transferer_address = ethrex_l2_sdk::get_address_from_secret_key(transferer_private_key)?;
    let transfer_value = U256::from(10000000000u128);

    let transferer_initial_l2_balance = proposer_client
        .get_balance(transferer_address, BlockByNumber::Latest)
        .await?;

    assert!(
        transferer_initial_l2_balance >= transfer_value,
        "L2 transferer doesn't have enough balance to transfer"
    );

    let transfer_recipient_initial_balance = proposer_client
        .get_balance(transfer_recipient_address, BlockByNumber::Latest)
        .await?;

    assert!(
        transfer_recipient_initial_balance.is_zero(),
        "L2 transfer recipient should have zero balance"
    );

    let fee_vault_balance_before_transfer = proposer_client
        .get_balance(fees_vault(), BlockByNumber::Latest)
        .await?;

    let transfer_tx = ethrex_l2_sdk::transfer(
        transfer_value,
        transferer_address,
        transfer_recipient_address,
        transferer_private_key,
        proposer_client,
    )
    .await?;

    let transfer_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(transfer_tx, proposer_client, 1000).await?;

    let recoverable_fees_vault_balance = proposer_client
        .get_balance(fees_vault(), BlockByNumber::Latest)
        .await?;

    println!("Recoverable Fees Balance: {recoverable_fees_vault_balance}",);

    println!("Checking balances on L2 after transfer");

    let transferer_l2_balance_after_transfer = proposer_client
        .get_balance(transferer_address, BlockByNumber::Latest)
        .await?;

    assert!(
        (transferer_initial_l2_balance - transfer_value).abs_diff(transferer_l2_balance_after_transfer)
            < L2_GAS_COST_MAX_DELTA,
        "L2 transferer balance didn't decrease as expected after transfer. Gas costs were {}/{L2_GAS_COST_MAX_DELTA}",
        (transferer_initial_l2_balance - transfer_value).abs_diff(transferer_l2_balance_after_transfer)
    );

    let transfer_recipient_l2_balance_after_transfer = proposer_client
        .get_balance(transfer_recipient_address, BlockByNumber::Latest)
        .await?;

    assert_eq!(
        transfer_recipient_l2_balance_after_transfer,
        transfer_recipient_initial_balance + transfer_value,
        "L2 transfer recipient balance didn't increase as expected after transfer"
    );

    let fee_vault_balance_after_transfer = proposer_client
        .get_balance(fees_vault(), BlockByNumber::Latest)
        .await?;

    let transfer_fees = get_fees_details_l2(transfer_tx_receipt, proposer_client).await;

    assert_eq!(
        fee_vault_balance_after_transfer,
        fee_vault_balance_before_transfer + transfer_fees.recoverable_fees,
        "Fee vault balance didn't increase as expected after transfer"
    );

    Ok(())
}

async fn test_n_withdraws(
    withdrawer_private_key: &SecretKey,
    eth_client: &EthClient,
    proposer_client: &EthClient,
    n: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    // 7. Withdraw funds from L2 to L1
    let withdrawer_address = ethrex_l2_sdk::get_address_from_secret_key(withdrawer_private_key)?;
    let withdraw_value = U256::from(100000000000000000000u128);

    println!("Checking balances on L1 and L2 before withdrawal");

    let withdrawer_l2_balance_before_withdrawal = proposer_client
        .get_balance(withdrawer_address, BlockByNumber::Latest)
        .await?;

    assert!(
        withdrawer_l2_balance_before_withdrawal >= withdraw_value,
        "L2 withdrawer doesn't have enough balance to withdraw"
    );

    let bridge_balance_before_withdrawal = eth_client
        .get_balance(common_bridge_address(), BlockByNumber::Latest)
        .await?;

    assert!(
        bridge_balance_before_withdrawal >= withdraw_value,
        "L1 bridge doesn't have enough balance to withdraw"
    );

    let withdrawer_l1_balance_before_withdrawal = eth_client
        .get_balance(withdrawer_address, BlockByNumber::Latest)
        .await?;

    let fee_vault_balance_before_withdrawal = proposer_client
        .get_balance(fees_vault(), BlockByNumber::Latest)
        .await?;

    println!("Withdrawing funds from L2 to L1");

    let mut withdraw_txs = vec![];
    let mut receipts = vec![];

    for x in 1..n + 1 {
        println!("Sending withdraw {x}/{n}");
        let withdraw_tx = ethrex_l2_sdk::withdraw(
            withdraw_value,
            withdrawer_address,
            *withdrawer_private_key,
            proposer_client,
        )
        .await?;

        withdraw_txs.push(withdraw_tx);

        let withdraw_tx_receipt =
            ethrex_l2_sdk::wait_for_transaction_receipt(withdraw_tx, proposer_client, 1000)
                .await
                .expect("Withdraw tx receipt not found");

        receipts.push(withdraw_tx_receipt);
    }

    println!("Checking balances on L1 and L2 after withdrawal");

    let withdrawer_l2_balance_after_withdrawal = proposer_client
        .get_balance(withdrawer_address, BlockByNumber::Latest)
        .await?;

    assert!(
        (withdrawer_l2_balance_before_withdrawal - withdraw_value * n)
            .abs_diff(withdrawer_l2_balance_after_withdrawal)
            < L2_GAS_COST_MAX_DELTA * n,
        "Withdrawer L2 balance didn't decrease as expected after withdrawal"
    );

    let withdrawer_l1_balance_after_withdrawal = eth_client
        .get_balance(withdrawer_address, BlockByNumber::Latest)
        .await?;

    assert_eq!(
        withdrawer_l1_balance_after_withdrawal, withdrawer_l1_balance_before_withdrawal,
        "Withdrawer L1 balance should not change after withdrawal"
    );

    let fee_vault_balance_after_withdrawal = proposer_client
        .get_balance(fees_vault(), BlockByNumber::Latest)
        .await?;

    let mut withdraw_fees = U256::zero();
    for receipt in receipts {
        withdraw_fees += get_fees_details_l2(receipt, proposer_client)
            .await
            .recoverable_fees;
    }

    assert_eq!(
        fee_vault_balance_after_withdrawal,
        fee_vault_balance_before_withdrawal + withdraw_fees,
        "Fee vault balance didn't increase as expected after withdrawal"
    );

    // We need to wait for all the txs to be included in some batch
    let mut proofs = vec![];
    for (i, tx) in withdraw_txs.clone().into_iter().enumerate() {
        println!("Getting withdrawal proof {}/{n}", i + 1);
        let withdrawal_proof = proposer_client.wait_for_withdrawal_proof(tx, 1000).await?;
        proofs.push(withdrawal_proof);
    }

    for proof in &proofs {
        while u64::from_str_radix(
            eth_client
                .call(
                    Address::from_str(
                        &std::env::var("ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS")
                            .expect("ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS env var not set"),
                    )
                    .unwrap(),
                    calldata::encode_calldata("lastVerifiedBatch()", &[])?.into(),
                    Overrides::default(),
                )
                .await?
                .get(2..)
                .unwrap(),
            16,
        )
        .unwrap()
            < proof.batch_number
        {
            println!("Withdrawal is not verified on L1 yet");
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    let mut withdraw_claim_txs_receipts = vec![];

    for (x, (tx, proof)) in withdraw_txs.iter().zip(proofs.iter()).enumerate() {
        println!("Claiming withdrawal on L1 {x}/{n}");

        let withdraw_claim_tx = ethrex_l2_sdk::claim_withdraw(
            withdraw_value,
            *tx,
            withdrawer_address,
            *withdrawer_private_key,
            eth_client,
            proof,
        )
        .await?;
        let withdraw_claim_tx_receipt =
            wait_for_transaction_receipt(withdraw_claim_tx, eth_client, 5).await?;
        withdraw_claim_txs_receipts.push(withdraw_claim_tx_receipt);
    }

    println!("Checking balances on L1 and L2 after claim");

    let withdrawer_l1_balance_after_claim = eth_client
        .get_balance(withdrawer_address, BlockByNumber::Latest)
        .await?;

    let gas_used_value: u64 = withdraw_claim_txs_receipts
        .iter()
        .map(|x| x.tx_info.gas_used * x.tx_info.effective_gas_price)
        .sum();

    assert_eq!(
        withdrawer_l1_balance_after_claim,
        withdrawer_l1_balance_after_withdrawal + withdraw_value * n - gas_used_value,
        "Withdrawer L1 balance wasn't updated as expected after claim"
    );

    let withdrawer_l2_balance_after_claim = proposer_client
        .get_balance(withdrawer_address, BlockByNumber::Latest)
        .await?;

    assert_eq!(
        withdrawer_l2_balance_after_claim, withdrawer_l2_balance_after_withdrawal,
        "Withdrawer L2 balance should not change after claim"
    );

    let bridge_balance_after_withdrawal = eth_client
        .get_balance(common_bridge_address(), BlockByNumber::Latest)
        .await?;

    assert_eq!(
        bridge_balance_after_withdrawal,
        bridge_balance_before_withdrawal - withdraw_value * n,
        "Bridge balance didn't decrease as expected after withdrawal"
    );

    Ok(())
}

async fn test_deploy(
    init_code: &[u8],
    deployer_private_key: &SecretKey,
    proposer_client: &EthClient,
) -> Result<Address, Box<dyn std::error::Error>> {
    println!("Deploying contract on L2");

    let deployer_address = ethrex_l2_sdk::get_address_from_secret_key(deployer_private_key)?;

    let deployer_balance_before_deploy = proposer_client
        .get_balance(deployer_address, BlockByNumber::Latest)
        .await?;

    let fee_vault_balance_before_deploy = proposer_client
        .get_balance(fees_vault(), BlockByNumber::Latest)
        .await?;

    let (deploy_tx_hash, contract_address) = proposer_client
        .deploy(
            deployer_address,
            *deployer_private_key,
            init_code.to_vec().into(),
            Overrides::default(),
        )
        .await?;

    let deploy_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(deploy_tx_hash, proposer_client, 5).await?;

    let deploy_fees = get_fees_details_l2(deploy_tx_receipt, proposer_client).await;

    let deployer_balance_after_deploy = proposer_client
        .get_balance(deployer_address, BlockByNumber::Latest)
        .await?;

    assert_eq!(
        deployer_balance_after_deploy,
        deployer_balance_before_deploy - deploy_fees.total_fees,
        "Deployer L2 balance didn't decrease as expected after deploy"
    );

    let fee_vault_balance_after_deploy = proposer_client
        .get_balance(fees_vault(), BlockByNumber::Latest)
        .await?;

    assert_eq!(
        fee_vault_balance_after_deploy,
        fee_vault_balance_before_deploy + deploy_fees.recoverable_fees,
        "Fee vault balance didn't increase as expected after deploy"
    );

    let deployed_contract_balance = proposer_client
        .get_balance(contract_address, BlockByNumber::Latest)
        .await?;

    assert!(
        deployed_contract_balance.is_zero(),
        "Deployed contract balance should be zero after deploy"
    );

    Ok(contract_address)
}

async fn test_call_to_contract_with_deposit(
    deployed_contract_address: Address,
    calldata_to_contract: Bytes,
    caller_private_key: &SecretKey,
    proposer_client: &EthClient,
    eth_client: &EthClient,
) -> Result<(), Box<dyn std::error::Error>> {
    let caller_address = ethrex_l2_sdk::get_address_from_secret_key(caller_private_key)
        .expect("Failed to get address");

    let deposit_recipient_address = Address::random();
    let deposit_value = U256::from(100000000000000000000u128);

    println!("Checking balances before call");

    let caller_l1_balance_before_call = eth_client
        .get_balance(caller_address, BlockByNumber::Latest)
        .await?;

    assert!(
        caller_l1_balance_before_call >= deposit_value,
        "Caller doesn't have enough balance on L1 to deposit"
    );

    let deployed_contract_balance_before_call = proposer_client
        .get_balance(deployed_contract_address, BlockByNumber::Latest)
        .await?;

    let recipient_balance_before_call = proposer_client
        .get_balance(deposit_recipient_address, BlockByNumber::Latest)
        .await?;

    let fee_vault_balance_before_call = proposer_client
        .get_balance(fees_vault(), BlockByNumber::Latest)
        .await?;

    println!("Calling contract on L2 with deposit");

    let l1_to_l2_tx_hash = ethrex_l2_sdk::send_l1_to_l2_tx(
        caller_address,
        Some(deposit_value),
        Some(21000 * 5),
        L1ToL2TransactionData::new(
            deployed_contract_address,
            deposit_recipient_address,
            21000 * 5,
            calldata_to_contract.clone(),
        ),
        &l1_rich_wallet_private_key(),
        common_bridge_address(),
        eth_client,
    )
    .await?;

    println!("Waiting for L1 to L2 transaction receipt on L1");

    let l1_to_l2_tx_receipt = wait_for_transaction_receipt(l1_to_l2_tx_hash, eth_client, 5).await?;

    println!("Waiting for L1 to L2 transaction receipt on L2");

    let call_receipt = wait_for_l2_deposit_receipt(
        l1_to_l2_tx_receipt.block_info.block_number,
        eth_client,
        proposer_client,
    )
    .await?;

    println!("Checking balances after call");

    let caller_l1_balance_after_call = eth_client
        .get_balance(caller_address, BlockByNumber::Latest)
        .await?;

    assert_eq!(
        caller_l1_balance_after_call,
        caller_l1_balance_before_call
            - deposit_value
            - l1_to_l2_tx_receipt.tx_info.gas_used
                * l1_to_l2_tx_receipt.tx_info.effective_gas_price,
        "Caller L1 balance didn't decrease as expected after call"
    );

    let deposit_recipient_l2_balance_after_call = proposer_client
        .get_balance(deposit_recipient_address, BlockByNumber::Latest)
        .await?;

    assert_eq!(
        deposit_recipient_l2_balance_after_call,
        recipient_balance_before_call + deposit_value,
        "Recipient L2 balance didn't increase as expected after call"
    );

    let fee_vault_balance_after_call = proposer_client
        .get_balance(fees_vault(), BlockByNumber::Latest)
        .await?;

    let call_fees = get_fees_details_l2(call_receipt, proposer_client).await;

    assert_eq!(
        fee_vault_balance_after_call,
        fee_vault_balance_before_call + call_fees.recoverable_fees,
        "Fee vault balance didn't increase as expected after call"
    );

    let deployed_contract_balance_after_call = proposer_client
        .get_balance(deployed_contract_address, BlockByNumber::Latest)
        .await?;

    assert_eq!(
        deployed_contract_balance_after_call, deployed_contract_balance_before_call,
        "Deployed contract increased unexpectedly after call"
    );

    Ok(())
}

// FIXME: Remove this before merging
#[allow(dead_code)]
#[derive(Debug)]
struct FeesDetails {
    total_fees: U256,
    recoverable_fees: U256,
    burned_fees: U256,
}

async fn get_fees_details_l2(tx_receipt: RpcReceipt, proposer_client: &EthClient) -> FeesDetails {
    let total_fees: U256 =
        (tx_receipt.tx_info.gas_used * tx_receipt.tx_info.effective_gas_price).into();

    let effective_gas_price = tx_receipt.tx_info.effective_gas_price;
    let base_fee_per_gas = proposer_client
        .get_block_by_number(BlockByNumber::Number(tx_receipt.block_info.block_number))
        .await
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    let max_priority_fee_per_gas_transfer: U256 = (effective_gas_price - base_fee_per_gas).into();

    let recoverable_fees = max_priority_fee_per_gas_transfer.mul(tx_receipt.tx_info.gas_used);

    FeesDetails {
        total_fees,
        recoverable_fees,
        burned_fees: total_fees - recoverable_fees,
    }
}

fn eth_client() -> EthClient {
    EthClient::new(DEFAULT_ETH_URL).unwrap()
}

fn proposer_client() -> EthClient {
    EthClient::new(DEFAULT_PROPOSER_URL).unwrap()
}

fn common_bridge_address() -> Address {
    std::env::var("ETHREX_WATCHER_BRIDGE_ADDRESS")
        .expect("ETHREX_WATCHER_BRIDGE_ADDRESS env var not set")
        .parse()
        .unwrap_or_else(|_| {
            println!(
                "ETHREX_WATCHER_BRIDGE_ADDRESS env var not set, using default: {DEFAULT_BRIDGE_ADDRESS}"
            );
            DEFAULT_BRIDGE_ADDRESS
        })
}

fn fees_vault() -> Address {
    DEFAULT_PROPOSER_COINBASE_ADDRESS
}

fn l1_rich_wallet_private_key() -> SecretKey {
    SecretKey::from_slice(DEFAULT_L1_RICH_WALLET_PRIVATE_KEY.as_bytes()).unwrap()
}

async fn wait_for_l2_deposit_receipt(
    l1_receipt_block_number: BlockNumber,
    eth_client: &EthClient,
    proposer_client: &EthClient,
) -> Result<RpcReceipt, Box<dyn std::error::Error>> {
    let topic =
        keccak(b"DepositInitiated(uint256,address,uint256,address,address,uint256,bytes,bytes32)");
    let logs = eth_client
        .get_logs(
            U256::from(l1_receipt_block_number),
            U256::from(l1_receipt_block_number),
            common_bridge_address(),
            topic,
        )
        .await?;

    let l2_deposit_tx_hash =
        H256::from_slice(logs.first().unwrap().log.data.get(128..160).unwrap());

    println!("Waiting for deposit transaction receipt on L2");

    Ok(
        ethrex_l2_sdk::wait_for_transaction_receipt(l2_deposit_tx_hash, proposer_client, 1000)
            .await?,
    )
}

pub fn read_env_file_by_config() {
    let env_file_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".env");
    let reader = BufReader::new(File::open(env_file_path).expect("Failed to open .env file"));

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
                std::env::set_var(key, value)
            }
            None => continue,
        };
    }
}
