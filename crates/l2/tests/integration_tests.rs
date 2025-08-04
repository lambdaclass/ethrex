#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use crate::harness::contracts::{test_privileged_tx_with_contract_call, test_privileged_tx_with_contract_call_revert, test_upgrade};
use crate::harness::erc20::{test_erc20_failed_deposit, test_erc20_roundtrip};
use crate::harness::eth::{test_aliasing, test_deposit, test_forced_withdrawal, test_gas_burning, test_5_withdrawals, test_privileged_spammer, test_privileged_tx_not_enough_balance, test_total_eth_l2, test_transfer, test_transfer_with_privileged_tx};
use crate::harness::{clean_contracts_dir, deposit, rich_pk_1, l1_client, l2_client, rich_pk_2, read_env_file_by_config};

mod harness;

 #[tokio::test]
 async fn l2_integration_test() -> Result<(), Box<dyn std::error::Error>> {
     read_env_file_by_config();

     test_upgrade().await?;

     test_deposit(
     )
     .await?;

     // this test should go before the withdrawal ones
     // it's failure case is making a batch invalid due to invalid privileged transactions
     test_privileged_spammer().await?;

     test_transfer(
     )
     .await?;

     test_transfer_with_privileged_tx(
     )
     .await?;

     test_gas_burning().await?;

     test_privileged_tx_with_contract_call().await?;

     test_privileged_tx_with_contract_call_revert()
         .await?;

     test_privileged_tx_not_enough_balance(
     )
     .await?;

     test_aliasing().await?;

     test_erc20_roundtrip().await?;

     test_erc20_failed_deposit().await?;

     test_forced_withdrawal().await?;

     test_5_withdrawals(
     )
     .await?;

     test_total_eth_l2().await?;

     clean_contracts_dir();

     println!("l2_integration_test is done");
     Ok(())
 }
