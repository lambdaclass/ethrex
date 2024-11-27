#![allow(clippy::unwrap_used)]
use bytes::Bytes;
use ethereum_types::{Address, H160, U256};
use ethrex_l2::utils::eth_client::{eth_sender::Overrides, EthClient};
use keccak_hash::H256;
use secp256k1::SecretKey;
use std::{str::FromStr, time::Duration};

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
async fn testito() {
    let eth_client = eth_client();
    let proposer_client = proposer_client();

    // 1. Check balances on L1 and L2

    println!("Checking initial balances on L1 and L2");

    let l1_initial_balance = eth_client
        .get_balance(l1_rich_wallet_address())
        .await
        .unwrap();
    let l2_initial_balance = proposer_client
        .get_balance(l1_rich_wallet_address())
        .await
        .unwrap();

    println!("L1 initial balance: {l1_initial_balance}");
    println!("L2 initial balance: {l2_initial_balance}");

    // 2. Deposit from L1 to L2

    println!("Depositing funds from L1 to L2");

    let deposit_value = U256::from(1000000000000000000000u128);
    let deposit_tx = ethrex_l2_sdk::deposit(
        deposit_value,
        l1_rich_wallet_address(),
        l1_rich_wallet_private_key(),
        &eth_client,
    )
    .await
    .unwrap();

    println!("Waiting for deposit transaction receipt");

    let _deposit_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(deposit_tx, &eth_client, 5)
            .await
            .unwrap();

    // 3. Check balances on L1 and L2

    println!("Checking balances on L1 and L2 after deposit");

    let l1_after_deposit_balance = eth_client
        .get_balance(l1_rich_wallet_address())
        .await
        .unwrap();
    let mut l2_after_deposit_balance = proposer_client
        .get_balance(l1_rich_wallet_address())
        .await
        .unwrap();

    println!("Waiting for L2 balance to update");

    // TODO: Improve this. Ideally, the L1 contract should return the L2 mint
    // tx hash for the user to wait for the receipt.
    let mut retries = 0;
    while retries < 30 && l2_after_deposit_balance < l2_initial_balance + deposit_value {
        std::thread::sleep(std::time::Duration::from_secs(2));
        println!("[{retries}/30] Waiting for L2 balance to update after deposit");
        l2_after_deposit_balance = proposer_client
            .get_balance(l1_rich_wallet_address())
            .await
            .unwrap();
        retries += 1;
    }

    assert_ne!(retries, 30, "L2 balance did not update after deposit");

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

    // 4. Transfer funds on L2

    println!("Transferring funds on L2");

    let (random_account_address, _random_account_private_key) = random_account();
    let l2_random_account_initial_balance = proposer_client
        .get_balance(random_account_address)
        .await
        .unwrap();
    assert!(l2_random_account_initial_balance.is_zero());
    let transfer_value = U256::from(10000000000u128);
    let transfer_tx = ethrex_l2_sdk::transfer(
        transfer_value,
        l1_rich_wallet_address(),
        random_account_address,
        l1_rich_wallet_private_key(),
        &proposer_client,
    )
    .await
    .unwrap();
    let _transfer_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(transfer_tx, &proposer_client, 30)
            .await
            .unwrap();

    // 5. Check balances on L2

    println!("Checking balances on L2 after transfer");

    let l2_balance_after_transfer = proposer_client
        .get_balance(l1_rich_wallet_address())
        .await
        .unwrap();
    let l2_random_account_balance_after_transfer = proposer_client
        .get_balance(random_account_address)
        .await
        .unwrap();

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
        l1_rich_wallet_address(),
        l1_rich_wallet_private_key(),
        &proposer_client,
    )
    .await
    .unwrap();
    let withdraw_tx_receipt =
        ethrex_l2_sdk::wait_for_transaction_receipt(withdraw_tx, &proposer_client, 30)
            .await
            .unwrap();

    // 7. Check balances on L1 and L2

    println!("Checking balances on L1 and L2 after withdrawal");

    let l1_after_withdrawal_balance = eth_client
        .get_balance(l1_rich_wallet_address())
        .await
        .unwrap();
    let l2_after_withdrawal_balance = proposer_client
        .get_balance(l1_rich_wallet_address())
        .await
        .unwrap();

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
        &eth_client
            .call(
                Address::from_str(
                    &std::env::var("ON_CHAIN_PROPOSER_ADDRESS")
                        .expect("ON_CHAIN_PROPOSER env var not set"),
                )
                .unwrap(),
                // lastVerifiedBlock()
                Bytes::from_static(&[0x2f, 0xde, 0x80, 0xe5]),
                Overrides::default(),
            )
            .await
            .unwrap()[2..],
        16,
    )
    .unwrap()
        < withdraw_tx_receipt.block_info.block_number
    {
        println!("Withdrawal is not verified on L1 yet");
        std::thread::sleep(Duration::from_secs(2));
    }

    let claim_tx = ethrex_l2_sdk::claim_withdraw(
        withdraw_tx,
        withdraw_value,
        l1_rich_wallet_address(),
        l1_rich_wallet_private_key(),
        &proposer_client,
        &eth_client,
    )
    .await
    .unwrap();

    let _claim_tx_receipt = ethrex_l2_sdk::wait_for_transaction_receipt(claim_tx, &eth_client, 15)
        .await
        .unwrap();

    // 9. Check balances on L1 and L2

    println!("Checking balances on L1 and L2 after claim");

    let l1_after_claim_balance = eth_client
        .get_balance(l1_rich_wallet_address())
        .await
        .unwrap();
    let l2_after_claim_balance = proposer_client
        .get_balance(l1_rich_wallet_address())
        .await
        .unwrap();

    println!("L1 balance after claim: {l1_after_claim_balance}");
    println!("L2 balance after claim: {l2_after_claim_balance}");

    assert!(
        (l1_after_withdrawal_balance + withdraw_value).abs_diff(l1_after_claim_balance)
            < L1_GAS_COST_MAX_DELTA,
        "L1 balance should have increased with withdraw value + gas costs"
    );
    assert_eq!(
        l2_after_withdrawal_balance, l2_after_claim_balance,
        "L2 balance should not change after claim"
    );
}

fn eth_client() -> EthClient {
    EthClient::new(&std::env::var("ETH_URL").unwrap_or(DEFAULT_ETH_URL.to_owned()))
}

fn proposer_client() -> EthClient {
    EthClient::new(&std::env::var("PROPOSER_URL").unwrap_or(DEFAULT_PROPOSER_URL.to_owned()))
}

#[allow(clippy::unwrap_used)]
fn l1_rich_wallet_address() -> Address {
    std::env::var("L1_RICH_WALLET_ADDRESS")
        .unwrap_or(format!("{DEFAULT_L1_RICH_WALLET_ADDRESS:#x}"))
        .parse()
        .unwrap()
}

#[allow(clippy::unwrap_used)]
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
