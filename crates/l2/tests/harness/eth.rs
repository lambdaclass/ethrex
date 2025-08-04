use crate::harness::{
    fees_vault, find_withdrawal_with_widget, get_fees_details_l2, rich_pk_1, get_rich_accounts_balance, l1_client, l2_client, perform_transfer, test_deploy_l1, test_send, transfer_value, wait_for_l2_deposit_receipt, wait_for_verified_proof, L2_GAS_COST_MAX_DELTA
};
use bytes::Bytes;
use color_eyre::eyre;
use ethrex_common::{
     H160, U256,
};
use ethrex_l2::monitor::widget::l2_to_l1_messages::{
    L2ToL1MessageKind, L2ToL1MessageRow, L2ToL1MessageStatus,
};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_sdk::{
    bridge_address, calldata::encode_calldata, claim_withdraw, get_address_alias, get_address_from_secret_key, l1_to_l2_tx_data::L1ToL2TransactionData, wait_for_transaction_receipt, COMMON_BRIDGE_L2_ADDRESS
};
use ethrex_rpc::{ types::block_identifier::{BlockIdentifier, BlockTag}};

use crate::harness::{deposit, rich_pk_2};

pub async fn test_deposit() -> eyre::Result<()> {
    let rich_wallet_private_key = rich_pk_1();
    deposit(
        &l1_client(),
        &l2_client(),
        rich_wallet_private_key,
        U256::from(42)
    ).await
}

/// Tests that a withdrawal can be triggered by a privileged transaction
/// This ensures the sequencer can't censor withdrawals without stopping the network
pub async fn test_forced_withdrawal() -> Result<(), Box<dyn std::error::Error>> {
    let l1_client = l1_client();
    let l2_client = l2_client();
    let rich_wallet_private_key = rich_pk_1();
    println!("Testing forced withdrawal");
    let rich_address = ethrex_l2_sdk::get_address_from_secret_key(&rich_wallet_private_key)
        .expect("Failed to get address");
    let l1_initial_balance = l1_client
        .get_balance(rich_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;
    let l2_initial_balance = l2_client
        .get_balance(rich_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;
    let transfer_value = U256::from(100);
    let mut l1_gas_costs = 0;

    let calldata = encode_calldata("withdraw(address)", &[Value::Address(rich_address)])?;

    println!("forced_withdrawal: Sending L1 to L2 transaction");

    let l1_to_l2_tx_hash = ethrex_l2_sdk::send_l1_to_l2_tx(
        rich_address,
        Some(0),
        None,
        L1ToL2TransactionData::new(
            COMMON_BRIDGE_L2_ADDRESS,
            21000 * 5,
            transfer_value,
            Bytes::from(calldata),
        ),
        &rich_pk_1(),
        bridge_address()?,
        &l1_client,
    )
    .await?;

    println!("forced_withdrawal: Waiting for L1 to L2 transaction receipt on L1");

    let l1_to_l2_tx_receipt =
        wait_for_transaction_receipt(l1_to_l2_tx_hash, &l1_client, 5).await?;

    assert!(l1_to_l2_tx_receipt.receipt.status);

    l1_gas_costs +=
        l1_to_l2_tx_receipt.tx_info.gas_used * l1_to_l2_tx_receipt.tx_info.effective_gas_price;
    println!("forced_withdrawal: Waiting for L1 to L2 transaction receipt on L2");

    let res = wait_for_l2_deposit_receipt(
        l1_to_l2_tx_receipt.block_info.block_number,
        &l1_client,
        &l2_client,
    )
    .await?;

    let withdrawal_tx_hash = res.tx_info.transaction_hash;
    assert_eq!(
        find_withdrawal_with_widget(bridge_address()?, withdrawal_tx_hash, &l2_client, &l1_client)
            .await
            .unwrap(),
        L2ToL1MessageRow {
            status: L2ToL1MessageStatus::WithdrawalInitiated,
            kind: L2ToL1MessageKind::ETHWithdraw,
            receiver: rich_address,
            token_l1: Default::default(),
            token_l2: Default::default(),
            value: transfer_value,
            l2_tx_hash: withdrawal_tx_hash
        }
    );

    let l2_final_balance = l2_client
        .get_balance(rich_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    println!("forced_withdrawal: Waiting for withdrawal proof on L2");
    let proof = wait_for_verified_proof(&l1_client, &l2_client, res.tx_info.transaction_hash).await;

    println!("forced_withdrawal: Claiming withdrawal on L1");

    let withdraw_claim_tx = claim_withdraw(
        transfer_value,
        rich_address,
        rich_wallet_private_key,
        &l1_client,
        &proof,
    )
    .await
    .expect("error while claiming");
    let res = wait_for_transaction_receipt(withdraw_claim_tx, &l1_client, 5).await?;
    l1_gas_costs += res.tx_info.gas_used * res.tx_info.effective_gas_price;
    assert_eq!(
        find_withdrawal_with_widget(bridge_address()?, withdrawal_tx_hash, &l2_client, &l1_client)
            .await
            .unwrap(),
        L2ToL1MessageRow {
            status: L2ToL1MessageStatus::WithdrawalClaimed,
            kind: L2ToL1MessageKind::ETHWithdraw,
            receiver: rich_address,
            token_l1: Default::default(),
            token_l2: Default::default(),
            value: transfer_value,
            l2_tx_hash: withdrawal_tx_hash
        }
    );

    let l1_final_balance = l1_client
        .get_balance(rich_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;
    assert_eq!(
        l1_initial_balance + transfer_value - l1_gas_costs,
        l1_final_balance
    );
    assert_eq!(l2_initial_balance - transfer_value, l2_final_balance);
    Ok(())
}

pub async fn test_privileged_spammer() -> Result<(), Box<dyn std::error::Error>> {
    let l1_client = l1_client();
    let rich_wallet_private_key = rich_pk_1();
    let init_code_l1 = hex::decode(std::fs::read(
        "../../fixtures/contracts/deposit_spammer/DepositSpammer.bin",
    )?)?;
    let caller_l1 = test_deploy_l1(&l1_client, &init_code_l1, &rich_wallet_private_key).await?;
    for _ in 0..10 {
        test_send(
            &l1_client,
            &rich_wallet_private_key,
            caller_l1,
            "spam(address,uint256)",
            &[Value::Address(bridge_address()?), Value::Uint(5.into())],
        )
        .await;
    }
    Ok(())
}

pub async fn test_transfer() -> Result<(), Box<dyn std::error::Error>> {
    let l2_client = l2_client();
    let transferer_private_key = rich_pk_1();
    let returnerer_private_key = rich_pk_2();
    println!("test transfer: Transferring funds on L2");
    let transferer_address = get_address_from_secret_key(&transferer_private_key)?;
    let returner_address = get_address_from_secret_key(&returnerer_private_key)?;

    println!(
        "test transfer: Performing transfer from {transferer_address:#x} to {returner_address:#x}"
    );

    perform_transfer(
        &l2_client,
        &transferer_private_key,
        returner_address,
        transfer_value(),
    )
    .await?;
    // Only return 99% of the transfer, other amount is for fees
    let return_amount = (transfer_value() * 99) / 100;

    println!(
        "test transfer: Performing return transfer from {returner_address:#x} to {transferer_address:#x} with amount {return_amount}"
    );

    perform_transfer(
        &l2_client,
        &returnerer_private_key,
        transferer_address,
        return_amount,
    )
    .await?;

    Ok(())
}

pub async fn test_transfer_with_privileged_tx() -> Result<(), Box<dyn std::error::Error>> {
    let l1_client = l1_client();
    let l2_client = l2_client();
    let transferer_private_key = rich_pk_1();
    let receiver_private_key = rich_pk_2();
    println!("transfer_with_ptx: Transferring funds on L2 through a deposit");
    let transferer_address = get_address_from_secret_key(&transferer_private_key)?;
    let receiver_address = get_address_from_secret_key(&receiver_private_key)?;

    println!("transfer_with_ptx: Fetching receiver's initial balance on L2");

    let receiver_balance_before = l2_client
        .get_balance(receiver_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    println!(
        "transfer_with_ptx: Performing transfer through deposit from {transferer_address:#x} to {receiver_address:#x}."
    );

    let l1_to_l2_tx_hash = ethrex_l2_sdk::send_l1_to_l2_tx(
        transferer_address,
        Some(0),
        None,
        L1ToL2TransactionData::new(receiver_address, 21000 * 5, transfer_value(), Bytes::new()),
        &transferer_private_key,
        bridge_address()?,
        &l1_client,
    )
    .await?;

    println!("transfer_with_ptx: Waiting for L1 to L2 transaction receipt on L1");

    let l1_to_l2_tx_receipt =
        wait_for_transaction_receipt(l1_to_l2_tx_hash, &l1_client, 5).await?;

    assert!(
        l1_to_l2_tx_receipt.receipt.status,
        "Transfer transaction failed"
    );

    println!("transfer_with_ptx: Waiting for L1 to L2 transaction receipt on L2");

    let _ = wait_for_l2_deposit_receipt(
        l1_to_l2_tx_receipt.block_info.block_number,
        &l1_client,
        &l2_client,
    )
    .await?;

    println!("transfer_with_ptx: Checking balances after transfer");

    let receiver_balance_after = l2_client
        .get_balance(receiver_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;
    assert_eq!(
        receiver_balance_after,
        receiver_balance_before + transfer_value()
    );
    Ok(())
}

pub async fn test_gas_burning() -> Result<(), Box<dyn std::error::Error>> {
    let l1_client = l1_client();
    let rich_wallet_private_key = rich_pk_1();
    println!("test_gas_burning: Transferring funds on L2 through a deposit");
    let rich_address = get_address_from_secret_key(&rich_wallet_private_key)?;
    let l2_gas_limit = 2_000_000;
    let l1_extra_gas_limit = 400_000;

    let l1_to_l2_tx_hash = ethrex_l2_sdk::send_l1_to_l2_tx(
        rich_address,
        Some(0),
        Some(l2_gas_limit + l1_extra_gas_limit),
        L1ToL2TransactionData::new(rich_address, l2_gas_limit, U256::zero(), Bytes::new()),
        &rich_wallet_private_key,
        bridge_address()?,
        &l1_client,
    )
    .await?;

    println!("test_gas_burning: Waiting for L1 to L2 transaction receipt on L1");

    let l1_to_l2_tx_receipt =
        wait_for_transaction_receipt(l1_to_l2_tx_hash, &l1_client, 5).await?;

    assert!(l1_to_l2_tx_receipt.receipt.status);
    assert!(l1_to_l2_tx_receipt.tx_info.gas_used > l2_gas_limit);
    assert!(l1_to_l2_tx_receipt.tx_info.gas_used < l2_gas_limit + l1_extra_gas_limit);
    Ok(())
}

pub async fn test_privileged_tx_not_enough_balance() -> Result<(), Box<dyn std::error::Error>> {
    let l1_client = l1_client();
    let l2_client = l2_client();
    let rich_wallet_private_key = rich_pk_1();
    let receiver_private_key = rich_pk_2();
    println!("Starting test for privileged transaction with insufficient balance");
    let rich_address = get_address_from_secret_key(&rich_wallet_private_key)?;
    let receiver_address = get_address_from_secret_key(&receiver_private_key)?;

    println!("ptx_not_enough_balance: Fetching initial balances on L1 and L2");

    let balance_sender = l2_client
        .get_balance(rich_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;
    let balance_before = l2_client
        .get_balance(receiver_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    let transfer_value = balance_sender + U256::one();

    println!(
        "ptx_not_enough_balance: Attempting to transfer {transfer_value} from {rich_address:#x} to {receiver_address:#x}"
    );

    let l1_to_l2_tx_hash = ethrex_l2_sdk::send_l1_to_l2_tx(
        rich_address,
        Some(0),
        None,
        L1ToL2TransactionData::new(receiver_address, 21000 * 5, transfer_value, Bytes::new()),
        &rich_wallet_private_key,
        bridge_address()?,
        &l1_client,
    )
    .await?;

    println!("ptx_not_enough_balance: Waiting for L1 to L2 transaction receipt on L1");

    let l1_to_l2_tx_receipt =
        wait_for_transaction_receipt(l1_to_l2_tx_hash, &l1_client, 5).await?;

    assert!(
        l1_to_l2_tx_receipt.receipt.status,
        "Transfer transaction failed"
    );

    println!("ptx_not_enough_balance: Waiting for L1 to L2 transaction receipt on L2");

    let _ = wait_for_l2_deposit_receipt(
        l1_to_l2_tx_receipt.block_info.block_number,
        &l1_client,
        &l2_client,
    )
    .await?;

    println!("ptx_not_enough_balance: Checking balances after transfer");

    let balance_after = l2_client
        .get_balance(receiver_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;
    assert_eq!(balance_after, balance_before);
    Ok(())
}


pub async fn test_5_withdrawals() -> Result<(), Box<dyn std::error::Error>> {
    let n = 5;
    let l1_client = l1_client();
    let l2_client = l2_client();
    let withdrawer_private_key = rich_pk_1();
    println!("test_n_withdraws: Withdrawing funds from L2 to L1");
    let withdrawer_address = ethrex_l2_sdk::get_address_from_secret_key(&withdrawer_private_key)?;
    let withdraw_value = std::env::var("INTEGRATION_TEST_WITHDRAW_VALUE")
        .map(|value| U256::from_dec_str(&value).expect("Invalid withdraw value"))
        .unwrap_or(U256::from(100000000000000000000u128));

    println!("test_n_withdraws: Checking balances on L1 and L2 before withdrawal");

    let withdrawer_l2_balance_before_withdrawal = l2_client
        .get_balance(withdrawer_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert!(
        withdrawer_l2_balance_before_withdrawal >= withdraw_value,
        "L2 withdrawer doesn't have enough balance to withdraw"
    );

    let bridge_balance_before_withdrawal = l1_client
        .get_balance(bridge_address()?, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert!(
        bridge_balance_before_withdrawal >= withdraw_value,
        "L1 bridge doesn't have enough balance to withdraw"
    );

    let withdrawer_l1_balance_before_withdrawal = l1_client
        .get_balance(withdrawer_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    let fee_vault_balance_before_withdrawal = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    println!("test_n_withdraws: Withdrawing funds from L2 to L1");

    let mut withdraw_txs = vec![];
    let mut receipts = vec![];

    for x in 1..n + 1 {
        println!("test_n_withdraws: Sending withdraw {x}/{n}");
        let withdraw_tx = ethrex_l2_sdk::withdraw(
            withdraw_value,
            withdrawer_address,
            withdrawer_private_key,
            &l2_client,
        )
        .await?;

        withdraw_txs.push(withdraw_tx);

        let withdraw_tx_receipt =
            ethrex_l2_sdk::wait_for_transaction_receipt(withdraw_tx, &l2_client, 1000)
                .await
                .expect("Withdraw tx receipt not found");

        receipts.push(withdraw_tx_receipt);
    }

    println!("test_n_withdraws: Checking balances on L1 and L2 after withdrawal");

    let withdrawer_l2_balance_after_withdrawal = l2_client
        .get_balance(withdrawer_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert!(
        (withdrawer_l2_balance_before_withdrawal - withdraw_value * n)
            .abs_diff(withdrawer_l2_balance_after_withdrawal)
            < L2_GAS_COST_MAX_DELTA * n,
        "Withdrawer L2 balance didn't decrease as expected after withdrawal"
    );

    let withdrawer_l1_balance_after_withdrawal = l1_client
        .get_balance(withdrawer_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert_eq!(
        withdrawer_l1_balance_after_withdrawal, withdrawer_l1_balance_before_withdrawal,
        "Withdrawer L1 balance should not change after withdrawal"
    );

    let fee_vault_balance_after_withdrawal = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    let mut withdraw_fees = U256::zero();
    for receipt in receipts {
        withdraw_fees += get_fees_details_l2(receipt, &l2_client).await.recoverable_fees;
    }

    assert_eq!(
        fee_vault_balance_after_withdrawal,
        fee_vault_balance_before_withdrawal + withdraw_fees,
        "Fee vault balance didn't increase as expected after withdrawal"
    );

    // We need to wait for all the txs to be included in some batch
    let mut proofs = vec![];
    for (i, tx) in withdraw_txs.clone().into_iter().enumerate() {
        println!("Getting proof for withdrawal {i}/{n} ({tx:x})");
        proofs.push(wait_for_verified_proof(&l1_client, &l2_client, tx).await);
    }

    let mut withdraw_claim_txs_receipts = vec![];

    for (x, proof) in proofs.iter().enumerate() {
        println!("Claiming withdrawal on L1 {x}/{n}");

        let withdraw_claim_tx = ethrex_l2_sdk::claim_withdraw(
            withdraw_value,
            withdrawer_address,
            withdrawer_private_key,
            &l1_client,
            proof,
        )
        .await?;
        let withdraw_claim_tx_receipt =
            wait_for_transaction_receipt(withdraw_claim_tx, &l1_client, 5).await?;
        withdraw_claim_txs_receipts.push(withdraw_claim_tx_receipt);
    }

    println!("test_n_withdraws: Checking balances on L1 and L2 after claim");

    let withdrawer_l1_balance_after_claim = l1_client
        .get_balance(withdrawer_address, BlockIdentifier::Tag(BlockTag::Latest))
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

    let withdrawer_l2_balance_after_claim = l2_client
        .get_balance(withdrawer_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert_eq!(
        withdrawer_l2_balance_after_claim, withdrawer_l2_balance_after_withdrawal,
        "Withdrawer L2 balance should not change after claim"
    );

    let bridge_balance_after_withdrawal = l1_client
        .get_balance(bridge_address()?, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    assert_eq!(
        bridge_balance_after_withdrawal,
        bridge_balance_before_withdrawal - withdraw_value * n,
        "Bridge balance didn't decrease as expected after withdrawal"
    );

    Ok(())
}

pub async fn test_total_eth_l2() -> Result<(), Box<dyn std::error::Error>> {
    let l1_client = l1_client();
    let l2_client = l2_client();
    println!("Checking total ETH on L2");

    println!("Fetching rich accounts balance on L2");
    let rich_accounts_balance = get_rich_accounts_balance(&l2_client)
        .await
        .expect("Failed to get rich accounts balance");

    let coinbase_balance = l2_client
        .get_balance(fees_vault(), BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    println!("Coinbase balance: {coinbase_balance}");

    let total_eth_on_l2 = rich_accounts_balance + coinbase_balance;

    println!("Total ETH on L2: {rich_accounts_balance} + {coinbase_balance} = {total_eth_on_l2}");

    println!("Checking locked ETH on CommonBridge");

    let bridge_address = bridge_address()?;
    let bridge_locked_eth = l1_client
        .get_balance(bridge_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await?;

    println!("Bridge locked ETH: {bridge_locked_eth}");

    assert!(
        total_eth_on_l2 <= bridge_locked_eth,
        "Total ETH on L2 ({total_eth_on_l2}) is greater than bridge locked ETH ({bridge_locked_eth})"
    );

    Ok(())
}

pub async fn test_aliasing() -> Result<(), Box<dyn std::error::Error>> {
    let l1_client = l1_client();
    let l2_client = l2_client();
    let rich_wallet_private_key = rich_pk_1();
    println!("Testing aliasing");
    let init_code_l1 = hex::decode(std::fs::read("../../fixtures/contracts/caller/Caller.bin")?)?;
    let caller_l1 = test_deploy_l1(&l1_client, &init_code_l1, &rich_wallet_private_key).await?;
    let send_to_l2_calldata = encode_calldata(
        "sendToL2((address,uint256,uint256,bytes))",
        &[Value::Tuple(vec![
            Value::Address(H160::zero()),
            Value::Uint(U256::from(100_000)),
            Value::Uint(U256::zero()),
            Value::Bytes(Bytes::new()),
        ])],
    )?;

    println!("test_aliasing: Sending call to L2");
    let receipt_l1 = test_send(
        &l1_client,
        &rich_wallet_private_key,
        caller_l1,
        "doCall(address,bytes)",
        &[
            Value::Address(bridge_address()?),
            Value::Bytes(send_to_l2_calldata.into()),
        ],
    )
    .await;

    assert!(receipt_l1.receipt.status);

    let receipt_l2 =
        wait_for_l2_deposit_receipt(receipt_l1.block_info.block_number, &l1_client, &l2_client)
            .await
            .unwrap();
    println!(
        "alising {:#x} to {:#x}",
        get_address_alias(caller_l1),
        receipt_l2.tx_info.from
    );
    assert_eq!(receipt_l2.tx_info.from, get_address_alias(caller_l1));
    Ok(())
}
