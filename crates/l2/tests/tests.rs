#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
use bytes::Bytes;
use ethereum_types::{Address, H160, U256};
use ethrex_l2::utils::config::{read_env_file_by_config, ConfigMode};
use ethrex_l2_sdk::calldata;
use ethrex_rpc::clients::eth::{
    eth_sender::Overrides, from_hex_string_to_u256, BlockByNumber, EthClient,
};
use ethrex_rpc::types::receipt::RpcReceipt;
use keccak_hash::H256;
use secp256k1::SecretKey;
use std::{ops::Mul, str::FromStr, time::Duration};

const DEFAULT_ETH_URL: &str = "http://localhost:8545";
const DEFAULT_PROPOSER_URL: &str = "http://localhost:1729";
// 0x8943545177806ed17b9f23f0a21ee5948ecaa776
const DEFAULT_L1_RICH_WALLET_ADDRESS: Address = H160([
    0x89, 0x43, 0x54, 0x51, 0x77, 0x80, 0x6e, 0xd1, 0x7b, 0x9f, 0x23, 0xf0, 0xa2, 0x1e, 0xe5, 0x94,
    0x8e, 0xca, 0xa7, 0x76,
]);
// 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31
const DEFAULT_L1_RICH_WALLET_PRIVATE_KEY: H256 = H256([
    0xbc, 0xdf, 0x20, 0x24, 0x9a, 0xbf, 0x0e, 0xd6, 0xd9, 0x44, 0xc0, 0x28, 0x8f, 0xad, 0x48, 0x9e,
    0x33, 0xf6, 0x6b, 0x39, 0x60, 0xd9, 0xe6, 0x22, 0x9c, 0x1c, 0xd2, 0x14, 0xed, 0x3b, 0xbe, 0x31,
]);

const L1_GAS_COST_MAX_DELTA: U256 = U256([1_000_000_000_000_000, 0, 0, 0]);
const L2_GAS_COST_MAX_DELTA: U256 = U256([100_000_000_000_000, 0, 0, 0]);

/// Test the full flow of depositing, transferring, and withdrawing funds
/// from L1 to L2 and back.
///
/// 1. Check balances on L1 and L2
/// 2. Deposit from L1 to L2
/// 3. Check balances on L1 and L2
/// 4. Transfer funds on L2
/// 5. Check balances on L2
/// 6. Withdraw funds from L2 to L1
/// 7. Check balances on L1 and L2
/// 8. Claim funds on L1
/// 9. Check balances on L1 and L2
#[tokio::test]
async fn l2_integration_test() -> Result<(), Box<dyn std::error::Error>> {
    let eth_client = eth_client();
    let proposer_client = proposer_client();

    read_env_file_by_config(ConfigMode::Sequencer)?;

    // 1. Check balances on L1 and L2

    println!("Checking initial balances on L1 and L2");
    let l1_rich_wallet_address = l1_rich_wallet_address();

    let l1_initial_balance = eth_client.get_balance(l1_rich_wallet_address).await?;
    let mut l2_initial_balance = proposer_client.get_balance(l1_rich_wallet_address).await?;
    println!("Waiting for L2 to update for initial deposit");
    let mut retries = 0;
    while retries < 30 && l2_initial_balance.is_zero() {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        println!("[{retries}/30] Waiting for L2 balance to update");
        l2_initial_balance = proposer_client.get_balance(l1_rich_wallet_address).await?;
        retries += 1;
    }
    assert_ne!(retries, 30, "L2 balance is zero");
    let common_bridge_initial_balance = eth_client.get_balance(common_bridge_address()).await?;

    println!("L1 initial balance: {l1_initial_balance}");
    println!("L2 initial balance: {l2_initial_balance}");
    println!("Common Bridge initial balance: {common_bridge_initial_balance}");

    let recoverable_fees_vault_balance = proposer_client.get_balance(fees_vault()).await?;
    println!(
        "Recoverable Fees Balance: {}",
        recoverable_fees_vault_balance
    );

    // 2. Deposit from L1 to L2

    println!("Depositing funds from L1 to L2");

    let deposit_value = U256::from(1000000000000000000000u128);
    let deposit_tx = ethrex_l2_sdk::deposit(
        deposit_value,
        l1_rich_wallet_address,
        l1_rich_wallet_private_key(),
        &eth_client,
    )
    .await?;

    println!("Waiting for deposit transaction receipt");

    let _deposit_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(deposit_tx, &eth_client, 5).await?;

    let recoverable_fees_vault_balance = proposer_client.get_balance(fees_vault()).await?;
    println!(
        "Recoverable Fees Balance: {}",
        recoverable_fees_vault_balance
    );

    // 3. Check balances on L1 and L2

    println!("Checking balances on L1 and L2 after deposit");

    let l1_after_deposit_balance = eth_client.get_balance(l1_rich_wallet_address).await?;
    let mut l2_after_deposit_balance = proposer_client.get_balance(l1_rich_wallet_address).await?;

    println!("Waiting for L2 balance to update");

    // TODO: Improve this. Ideally, the L1 contract should return the L2 mint
    // tx hash for the user to wait for the receipt.
    let mut retries = 0;
    while retries < 30 && l2_after_deposit_balance < l2_initial_balance + deposit_value {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        println!("[{retries}/30] Waiting for L2 balance to update after deposit");
        l2_after_deposit_balance = proposer_client.get_balance(l1_rich_wallet_address).await?;
        retries += 1;
    }

    assert_ne!(retries, 30, "L2 balance did not update after deposit");

    let common_bridge_locked_balance = eth_client.get_balance(common_bridge_address()).await?;
    // Check that the deposit amount is the amount locked by the CommonBridge
    assert_eq!(
        common_bridge_locked_balance,
        common_bridge_initial_balance + deposit_value
    );

    println!("L2 deposit received");

    println!("L1 balance after deposit: {l1_after_deposit_balance}");
    println!("L2 balance after deposit: {l2_after_deposit_balance}");

    assert_eq!(
        l2_initial_balance + deposit_value,
        l2_after_deposit_balance,
        "L2 balance should increase with deposit value"
    );
    assert!(
        (l1_initial_balance - deposit_value).abs_diff(l1_after_deposit_balance)
            < L1_GAS_COST_MAX_DELTA,
        "L1 balance should decrease with deposit value + gas costs. Gas costs were {}/{L1_GAS_COST_MAX_DELTA}",
        (l1_initial_balance - deposit_value).abs_diff(l1_after_deposit_balance)
    );

    let first_deposit_recoverable_fees_vault_balance =
        proposer_client.get_balance(fees_vault()).await?;
    println!(
        "Recoverable Fees Balance: {}, This amount is given because of the L2 Privileged Transaction, a deposit shouldn't give a tip to the coinbase address if the gas sent as tip doesn't come from the L1.",
        first_deposit_recoverable_fees_vault_balance
    );
    // 4. Transfer funds on L2

    println!("Transferring funds on L2");

    let (random_account_address, _random_account_private_key) = random_account();
    let l2_random_account_initial_balance =
        proposer_client.get_balance(random_account_address).await?;
    assert!(l2_random_account_initial_balance.is_zero());
    let transfer_value = U256::from(10000000000u128);
    let transfer_tx = ethrex_l2_sdk::transfer(
        transfer_value,
        l1_rich_wallet_address,
        random_account_address,
        l1_rich_wallet_private_key(),
        &proposer_client,
    )
    .await?;
    let transfer_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(transfer_tx, &proposer_client, 30).await?;

    let recoverable_fees_vault_balance = proposer_client.get_balance(fees_vault()).await?;
    println!(
        "Recoverable Fees Balance: {}",
        recoverable_fees_vault_balance
    );

    // 5. Check balances on L2

    println!("Checking balances on L2 after transfer");

    let l2_balance_after_transfer = proposer_client.get_balance(l1_rich_wallet_address).await?;
    let l2_random_account_balance_after_transfer =
        proposer_client.get_balance(random_account_address).await?;

    println!("L2 balance after transfer: {l2_balance_after_transfer}");
    println!("Random account balance after transfer: {l2_random_account_balance_after_transfer}");

    assert!(
        (l2_after_deposit_balance - transfer_value).abs_diff(l2_balance_after_transfer)
            < L2_GAS_COST_MAX_DELTA,
        "L2 balance should be decrease with transfer value + gas costs. Gas costs were {}/{L2_GAS_COST_MAX_DELTA}",
        (l2_after_deposit_balance - transfer_value).abs_diff(l2_balance_after_transfer)
    );
    assert_eq!(
        l2_random_account_initial_balance + transfer_value,
        l2_random_account_balance_after_transfer,
        "Random account balance should increase with transfer value"
    );

    // 6. Withdraw funds from L2 to L1

    println!("Withdrawing funds from L2 to L1");
    let withdraw_value = U256::from(100000000000000000000u128);
    let withdraw_tx = ethrex_l2_sdk::withdraw(
        withdraw_value,
        l1_rich_wallet_address,
        l1_rich_wallet_private_key(),
        &proposer_client,
    )
    .await?;
    let withdraw_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(withdraw_tx, &proposer_client, 30)
            .await
            .expect("Withdraw tx receipt not found");

    // 7. Check balances on L1 and L2

    println!("Checking balances on L1 and L2 after withdrawal");

    let l1_after_withdrawal_balance = eth_client.get_balance(l1_rich_wallet_address).await?;
    let l2_after_withdrawal_balance = proposer_client.get_balance(l1_rich_wallet_address).await?;

    println!("L1 balance after withdrawal: {l1_after_withdrawal_balance}");
    println!("L2 balance after withdrawal: {l2_after_withdrawal_balance}");

    assert_eq!(
        l1_after_deposit_balance, l1_after_withdrawal_balance,
        "L1 balance should not change after withdrawal"
    );
    assert!(
        (l2_balance_after_transfer - withdraw_value).abs_diff(l2_after_withdrawal_balance)
            < L2_GAS_COST_MAX_DELTA,
        "L2 balance should decrease with withdraw value + gas costs"
    );

    // 8. Claim funds on L1

    println!("Claiming funds on L1");

    while u64::from_str_radix(
        eth_client
            .call(
                Address::from_str(
                    &std::env::var("COMMITTER_ON_CHAIN_PROPOSER_ADDRESS")
                        .expect("ON_CHAIN_PROPOSER env var not set"),
                )
                .unwrap(),
                // lastVerifiedBlock()
                Bytes::from_static(&[0x2f, 0xde, 0x80, 0xe5]),
                Overrides::default(),
            )
            .await?
            .get(2..)
            .unwrap(),
        16,
    )
    .unwrap()
        < withdraw_tx_receipt.block_info.block_number
    {
        println!("Withdrawal is not verified on L1 yet");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    let claim_tx = ethrex_l2_sdk::claim_withdraw(
        withdraw_tx,
        withdraw_value,
        l1_rich_wallet_address,
        l1_rich_wallet_private_key(),
        &proposer_client,
        &eth_client,
    )
    .await?;

    let _claim_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(claim_tx, &eth_client, 15).await?;

    // 9. Check balances on L1 and L2

    println!("Checking balances on L1 and L2 after claim");

    let l1_after_claim_balance = eth_client.get_balance(l1_rich_wallet_address).await?;
    let l2_after_claim_balance = proposer_client.get_balance(l1_rich_wallet_address).await?;

    println!("L1 balance after claim: {l1_after_claim_balance}");
    println!("L2 balance after claim: {l2_after_claim_balance}");

    let common_bridge_locked_balance = eth_client.get_balance(common_bridge_address()).await?;
    let recoverable_fees_vault_balance = proposer_client.get_balance(fees_vault()).await?;
    println!(
        "Recoverable Fees Balance: {}",
        recoverable_fees_vault_balance
    );

    let fees_transfer = get_fees_details_l2(transfer_tx_receipt, &proposer_client).await;
    println!("transfer: {fees_transfer:?}");
    let fees_withdraw = get_fees_details_l2(withdraw_tx_receipt, &proposer_client).await;
    println!("withdraw: {fees_withdraw:?}");

    println!("Common Bridge Locked Balance: {common_bridge_locked_balance}");

    let total_locked_l2_value =
        deposit_value - withdraw_value - fees_transfer.total_fees - fees_withdraw.total_fees;
    let total_locked_l2_value_with_recoverable_fees =
        total_locked_l2_value + fees_transfer.recoverable_fees + fees_withdraw.recoverable_fees;

    let total_burned_fees = fees_transfer.burned_fees + fees_withdraw.burned_fees;
    println!("TOTAL Locked L2 value: {total_locked_l2_value}");
    println!(
        "TOTAL Locked L2 value with recoverable fees: {total_locked_l2_value_with_recoverable_fees}"
    );
    println!("BURNED FEES L2: {total_burned_fees}");

    println!("The total locked value by the CommonBridge contract doesn't take burned fees into account, also the deposit transactions \"gives\" some tokens (from fees) to the coinbase address. This behavior shouldn't happen.");

    // Check that we only have the amount left after the withdrawal
    assert_eq!(
        common_bridge_locked_balance,
        common_bridge_initial_balance + deposit_value - withdraw_value,
        "Amount after withdrawal differs"
    );

    // Check that the total_locked_l2_value_with_recoverable_fees matches the common_bridge_locked_balance - burned_fees
    // Check that we only have the amount left after the withdrawal
    assert_eq!(
        common_bridge_locked_balance,
        common_bridge_initial_balance
            + total_locked_l2_value_with_recoverable_fees
            + total_burned_fees,
        "Amount calculated after withdrawal differs"
    );

    // Check that the recoverable fees matches
    assert_eq!(
        recoverable_fees_vault_balance - first_deposit_recoverable_fees_vault_balance,
        fees_transfer.recoverable_fees + fees_withdraw.recoverable_fees,
        "Recoverable fees don't match"
    );

    assert!(
        (l1_after_withdrawal_balance + withdraw_value).abs_diff(l1_after_claim_balance)
            < L1_GAS_COST_MAX_DELTA,
        "L1 balance should have increased with withdraw value + gas costs"
    );
    assert_eq!(
        l2_after_withdrawal_balance, l2_after_claim_balance,
        "L2 balance should not change after claim"
    );

    println!("l2_integration_test is done");
    Ok(())
}

#[tokio::test]
async fn l2_sdk_deploy() -> Result<(), Box<dyn std::error::Error>> {
    let eth_client = eth_client();

    //pragma solidity ^0.8.27;
    //contract Test {
    //    uint256 public constant number = 37;
    //}
    let init_code = hex::decode("6080604052348015600e575f5ffd5b5060ac80601a5f395ff3fe6080604052348015600e575f5ffd5b50600436106026575f3560e01c80638381f58a14602a575b5f5ffd5b60306044565b604051603b9190605f565b60405180910390f35b602581565b5f819050919050565b6059816049565b82525050565b5f60208201905060705f8301846052565b9291505056fea2646970667358221220a6516c1bfca94ad11d1315b32cd08f115c050e098a0631d58ee55923e70bc36364736f6c634300081c0033")?;

    let (_, contract_address) = eth_client
        .deploy(
            l1_rich_wallet_address(),
            l1_rich_wallet_private_key(),
            init_code.into(),
            Overrides::default(),
        )
        .await?;

    let calldata: Bytes = calldata::encode_calldata("number()", &[])?.into();

    let hex_str = eth_client
        .call(contract_address, calldata, Overrides::default())
        .await?;
    let number = from_hex_string_to_u256(&hex_str)?.as_u64();

    assert_eq!(number, 37);
    Ok(())
}

#[derive(Debug)]
struct FeesDetails {
    total_fees: U256,
    recoverable_fees: U256,
    burned_fees: U256,
}

async fn get_fees_details_l2(tx_receipt: RpcReceipt, l2_client: &EthClient) -> FeesDetails {
    let total_fees: U256 =
        (tx_receipt.tx_info.gas_used * tx_receipt.tx_info.effective_gas_price).into();

    let effective_gas_price = tx_receipt.tx_info.effective_gas_price;
    let base_fee_per_gas = l2_client
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
    EthClient::new(&std::env::var("ETH_URL").unwrap_or(DEFAULT_ETH_URL.to_owned()))
}

fn proposer_client() -> EthClient {
    EthClient::new(&std::env::var("PROPOSER_URL").unwrap_or(DEFAULT_PROPOSER_URL.to_owned()))
}

fn l1_rich_wallet_address() -> Address {
    std::env::var("L1_RICH_WALLET_ADDRESS")
        .unwrap_or(format!("{DEFAULT_L1_RICH_WALLET_ADDRESS:#x}"))
        .parse()
        .unwrap()
}

fn common_bridge_address() -> Address {
    std::env::var("L1_WATCHER_BRIDGE_ADDRESS")
        .expect("L1_WATCHER_BRIDGE_ADDRESS env var not set")
        .parse()
        .unwrap()
}

fn fees_vault() -> Address {
    std::env::var("PROPOSER_COINBASE_ADDRESS")
        .expect("PROPOSER_COINBASE_ADDRESS env var not set")
        .parse()
        .unwrap()
}

fn l1_rich_wallet_private_key() -> SecretKey {
    std::env::var("L1_RICH_WALLET_PRIVATE_KEY")
        .map(|s| SecretKey::from_slice(H256::from_str(&s).unwrap().as_bytes()).unwrap())
        .unwrap_or(SecretKey::from_slice(DEFAULT_L1_RICH_WALLET_PRIVATE_KEY.as_bytes()).unwrap())
}

fn random_account() -> (Address, SecretKey) {
    let (sk, pk) = secp256k1::generate_keypair(&mut rand::thread_rng());
    let address = Address::from(keccak_hash::keccak(&pk.serialize_uncompressed()[1..]));
    (address, sk)
}
