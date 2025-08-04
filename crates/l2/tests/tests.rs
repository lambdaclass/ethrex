#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
use bytes::Bytes;
use ethrex_common::types::BlockNumber;
use ethrex_common::{Address, H160, H256, U256};
use ethrex_l2::monitor::widget::l2_to_l1_messages::{L2ToL1MessageKind, L2ToL1MessageStatus};
use ethrex_l2::monitor::widget::{L2ToL1MessagesTable, l2_to_l1_messages::L2ToL1MessageRow};
use ethrex_l2::sequencer::l1_watcher::PrivilegedTransactionData;
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::{
    clients::{deploy, send_eip1559_transaction},
    signer::{LocalSigner, Signer},
};
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_l2_sdk::l1_to_l2_tx_data::L1ToL2TransactionData;
use ethrex_l2_sdk::{
    COMMON_BRIDGE_L2_ADDRESS, bridge_address, claim_erc20withdraw, claim_withdraw,
    compile_contract, deposit_erc20, get_address_alias, get_address_from_secret_key,
    get_erc1967_slot, git_clone, wait_for_transaction_receipt,
};
use ethrex_rpc::{
    clients::eth::{EthClient, L1MessageProof, eth_sender::Overrides, from_hex_string_to_u256},
    types::{
        block_identifier::{BlockIdentifier, BlockTag},
        receipt::RpcReceipt,
    },
};
use hex::FromHexError;
use keccak_hash::keccak;
use rand::random;
use secp256k1::SecretKey;
use std::{
    fs::{File, read_to_string},
    io::{BufRead, BufReader},
    ops::Mul,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use crate::common::deposit;

mod common;

/// Test the full flow of depositing, depositing with contract call, transferring, and withdrawing funds
/// from L1 to L2 and back.
/// The test can be configured with the following environment variables
///
/// RPC urls:
/// INTEGRATION_TEST_L1_RPC: The url of the l1 rpc server
/// INTEGRATION_TEST_L2_RPC: The url of the l2 rpc server
///
/// Accounts private keys:
/// INTEGRATION_TEST_L1_RICH_WALLET_PRIVATE_KEY: The l1 private key that will make the deposit to the l2 and the transfer to the second l2 account
/// INTEGRATION_TEST_RETURN_TRANSFER_PRIVATE_KEY: The l2 private key that will receive the deposit and the transfer it back to the L1_RICH_WALLET_PRIVATE_KEY
/// ETHREX_DEPLOYER_PRIVATE_KEYS_FILE_PATH: The path to a file with pks that are rich accounts in the l2
///
/// Contract addresses:
/// ETHREX_WATCHER_BRIDGE_ADDRESS: The address of the l1 bridge contract
/// INTEGRATION_TEST_PROPOSER_COINBASE_ADDRESS: The address of the l2 coinbase
///
/// Test parameters:
///
/// INTEGRATION_TEST_DEPOSIT_VALUE: amount in wei to deposit from L1_RICH_WALLET_PRIVATE_KEY to the l2, this amount will be deposited 3 times over the course of the test
/// INTEGRATION_TEST_TRANSFER_VALUE: amount in wei to transfer to INTEGRATION_TEST_RETURN_TRANSFER_PRIVATE_KEY, this amount will be returned to the account
/// INTEGRATION_TEST_WITHDRAW_VALUE: amount in wei to withdraw from the l2 back to the l1 from L1_RICH_WALLET_PRIVATE_KEY this will be done INTEGRATION_TEST_WITHDRAW_COUNT times
/// INTEGRATION_TEST_WITHDRAW_COUNT: amount of withdraw transactions to send
/// INTEGRATION_TEST_SKIP_TEST_TOTAL_ETH: if set the integration test will not check for total eth in the chain, only to be used if we don't know all the accounts that exist in l2


const DEFAULT_PRIVATE_KEYS_FILE_PATH: &str = "../../fixtures/keys/private_keys_l1.txt";

// #[tokio::test]
// async fn l2_integration_test() -> Result<(), Box<dyn std::error::Error>> {
//     read_env_file_by_config();
//
//     let l1_client = l1_client();
//     let l2_client = l2_client();
//     let rich_wallet_private_key = l1_rich_wallet_private_key();
//     let transfer_return_private_key = l2_return_transfer_private_key();
//     let deposit_recipient_address = get_address_from_secret_key(&rich_wallet_private_key)
//         .expect("Failed to get address from l1 rich wallet pk");
//
//     test_upgrade(&l1_client, &l2_client).await?;
//
//     test_deposit(
//         &l1_client,
//         &l2_client,
//         &rich_wallet_private_key,
//         deposit_recipient_address,
//     )
//     .await?;
//
//     // this test should go before the withdrawal ones
//     // it's failure case is making a batch invalid due to invalid privileged transactions
//     test_privileged_spammer(&l1_client).await?;
//
//     test_transfer(
//         &l2_client,
//         &rich_wallet_private_key,
//         &transfer_return_private_key,
//     )
//     .await?;
//
//     test_transfer_with_privileged_tx(
//         &l1_client,
//         &l2_client,
//         &rich_wallet_private_key,
//         &transfer_return_private_key,
//     )
//     .await?;
//
//     test_gas_burning(&l1_client, &rich_wallet_private_key).await?;
//
//     test_privileged_tx_with_contract_call(&l1_client, &l2_client, &rich_wallet_private_key).await?;
//
//     test_privileged_tx_with_contract_call_revert(&l1_client, &l2_client, &rich_wallet_private_key)
//         .await?;
//
//     test_privileged_tx_not_enough_balance(
//         &l1_client,
//         &l2_client,
//         &rich_wallet_private_key,
//         &transfer_return_private_key,
//     )
//     .await?;
//
//     test_aliasing(&l1_client, &l2_client, &rich_wallet_private_key).await?;
//
//     test_erc20_roundtrip(&l1_client, &l2_client, &rich_wallet_private_key).await?;
//
//     test_erc20_failed_deposit(&l1_client, &l2_client, &rich_wallet_private_key).await?;
//
//     test_forced_withdrawal(&l1_client, &l2_client, &rich_wallet_private_key).await?;
//
//     let withdrawals_count = std::env::var("INTEGRATION_TEST_WITHDRAW_COUNT")
//         .map(|amount| amount.parse().expect("Invalid withdrawal amount value"))
//         .unwrap_or(5);
//
//     test_n_withdraws(
//         &l1_client,
//         &l2_client,
//         &rich_wallet_private_key,
//         withdrawals_count,
//     )
//     .await?;
//
//     if std::env::var("INTEGRATION_TEST_SKIP_TEST_TOTAL_ETH").is_err() {
//         test_total_eth_l2(&l1_client, &l2_client).await?;
//     }
//
//     clean_contracts_dir();
//
//     println!("l2_integration_test is done");
//     Ok(())
// }
//
