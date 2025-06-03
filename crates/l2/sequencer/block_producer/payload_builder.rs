use std::collections::HashMap;
use std::sync::Arc;

use ethrex_blockchain::{
    constants::TX_GAS_COST,
    error::ChainError,
    payload::{HeadTransaction, PayloadBuildContext, PayloadBuildResult},
    Blockchain,
};
use ethrex_common::{
    types::{Block, Receipt, Transaction, SAFE_BYTES_PER_BLOB},
    Address, U256,
};
use ethrex_metrics::metrics;
#[cfg(feature = "metrics")]
use ethrex_metrics::{
    metrics_blocks::METRICS_BLOCKS,
    metrics_transactions::{MetricsTxStatus, MetricsTxType, METRICS_TX},
};
use ethrex_storage::Store;
use ethrex_vm::{backends::CallFrameBackup, Evm, EvmError};
use std::ops::Div;
use tokio::time::Instant;
use tracing::{debug, error};

use crate::{
    sequencer::{
        errors::{BlockProducerError, StateDiffError},
        state_diff::{
            AccountStateDiff, BLOCK_HEADER_LEN, DEPOSITS_LOG_LEN, SIMPLE_TX_STATE_DIFF_SIZE,
            WITHDRAWAL_LOG_LEN,
        },
    },
    utils::helpers::{is_deposit_l2, is_withdrawal_l2},
};

/// L2 payload builder
/// Completes the payload building process, return the block value
/// Same as `blockchain::build_payload` without applying system operations and using a different `fill_transactions`
pub async fn build_payload(
    blockchain: Arc<Blockchain>,
    payload: Block,
    store: &Store,
) -> Result<PayloadBuildResult, BlockProducerError> {
    let since = Instant::now();
    let gas_limit = payload.header.gas_limit;

    debug!("Building payload");
    let mut context = PayloadBuildContext::new(payload, blockchain.evm_engine, store)?;

    blockchain.apply_withdrawals(&mut context)?;
    fill_transactions(blockchain.clone(), &mut context, store).await?;
    blockchain.extract_requests(&mut context)?;
    blockchain.finalize_payload(&mut context).await?;

    let interval = Instant::now().duration_since(since).as_millis();
    tracing::info!("[METRIC] BUILDING PAYLOAD TOOK: {interval} ms");
    #[allow(clippy::as_conversions)]
    if let Some(gas_used) = gas_limit.checked_sub(context.remaining_gas) {
        let as_gigas = (gas_used as f64).div(10_f64.powf(9_f64));

        if interval != 0 {
            let throughput = (as_gigas) / (interval as f64) * 1000_f64;
            tracing::info!(
                "[METRIC] BLOCK BUILDING THROUGHPUT: {throughput} Gigagas/s TIME SPENT: {interval} msecs"
            );
        }
    }

    metrics!(
        #[allow(clippy::as_conversions)]
        METRICS_BLOCKS.set_latest_block_gas_limit(
            ((gas_limit - context.remaining_gas) as f64 / gas_limit as f64) * 100_f64
        );
        // L2 does not allow for blob transactions so the blob pool can be ignored
        let (tx_pool_size, _blob_pool_size) = blockchain
            .mempool
            .get_mempool_size()
            .inspect_err(|e| tracing::error!("Failed to get metrics for: mempool size {}", e.to_string()))
            .unwrap_or((0_usize, 0_usize));
        let _ = METRICS_TX
            .set_mempool_tx_count(tx_pool_size, false)
            .inspect_err(|e| tracing::error!("Failed to set metrics for: blob tx mempool size {}", e.to_string()));
    );

    Ok(context.into())
}

/// Same as `blockchain::fill_transactions` but enforces that the `StateDiff` size  
/// stays within the blob size limit after processing each transaction.
pub async fn fill_transactions(
    blockchain: Arc<Blockchain>,
    context: &mut PayloadBuildContext,
    store: &Store,
) -> Result<(), BlockProducerError> {
    // version (u8) + header fields (struct) + withdrawals_len (u16) + deposits_len (u16) + accounts_diffs_len (u16)
    let mut acc_size_without_accounts = 1 + *BLOCK_HEADER_LEN + 2 + 2 + 2;
    let mut size_accounts_diffs = 0;
    let mut account_diffs = HashMap::new();

    let chain_config = store.get_chain_config()?;
    let max_blob_number_per_block: usize = chain_config
        .get_fork_blob_schedule(context.payload.header.timestamp)
        .map(|schedule| schedule.max)
        .unwrap_or_default()
        .try_into()
        .unwrap_or_default();

    debug!("Fetching transactions from mempool");
    // Fetch mempool transactions
    let latest_block_number = store.get_latest_block_number().await?;
    let (mut plain_txs, mut blob_txs) = blockchain.fetch_mempool_transactions(context)?;
    // Execute and add transactions to payload (if suitable)
    loop {
        // Check if we have enough gas to run more transactions
        if context.remaining_gas < TX_GAS_COST {
            debug!("No more gas to run transactions");
            break;
        };

        // Check if we have enough space for the StateDiff to run more transactions
        if acc_size_without_accounts + size_accounts_diffs + SIMPLE_TX_STATE_DIFF_SIZE
            > SAFE_BYTES_PER_BLOB
        {
            debug!("No more StateDiff space to run transactions");
            break;
        };
        if !blob_txs.is_empty() && context.blobs_bundle.blobs.len() >= max_blob_number_per_block {
            debug!("No more blob gas to run blob transactions");
            blob_txs.clear();
        }
        // Fetch the next transactions
        let (head_tx, is_blob) = match (plain_txs.peek(), blob_txs.peek()) {
            (None, None) => break,
            (None, Some(tx)) => (tx, true),
            (Some(tx), None) => (tx, false),
            (Some(a), Some(b)) if b < a => (b, true),
            (Some(tx), _) => (tx, false),
        };

        let txs = if is_blob {
            &mut blob_txs
        } else {
            &mut plain_txs
        };

        // Check if we have enough gas to run the transaction
        if context.remaining_gas < head_tx.tx.gas_limit() {
            debug!(
                "Skipping transaction: {}, no gas left",
                head_tx.tx.compute_hash()
            );
            // We don't have enough gas left for the transaction, so we skip all txs from this account
            txs.pop();
            continue;
        }

        // TODO: maybe fetch hash too when filtering mempool so we don't have to compute it here (we can do this in the same refactor as adding timestamp)
        let tx_hash = head_tx.tx.compute_hash();

        // Check whether the tx is replay-protected
        if head_tx.tx.protected() && !chain_config.is_eip155_activated(context.block_number()) {
            // Ignore replay protected tx & all txs from the sender
            // Pull transaction from the mempool
            debug!("Ignoring replay-protected transaction: {}", tx_hash);
            txs.pop();
            blockchain.remove_transaction_from_pool(&tx_hash)?;
            continue;
        }

        let maybe_sender_acc_info = store
            .get_account_info(latest_block_number, head_tx.tx.sender())
            .await?;

        if let Some(acc_info) = maybe_sender_acc_info {
            if head_tx.nonce() < acc_info.nonce {
                debug!("Removing transaction with nonce too low from mempool: {tx_hash:#x}");
                txs.pop();
                blockchain.remove_transaction_from_pool(&tx_hash)?;
                continue;
            }
        }

        // Execute tx
        let (receipt, transaction_backup) = match apply_transaction_l2(&head_tx, context) {
            Ok((receipt, transaction_backup)) => {
                metrics!(METRICS_TX.inc_tx_with_status_and_type(
                    MetricsTxStatus::Succeeded,
                    MetricsTxType(head_tx.tx_type())
                ));
                (receipt, transaction_backup)
            }
            // Ignore following txs from sender
            Err(e) => {
                debug!("Failed to execute transaction: {}, {e}", tx_hash);
                metrics!(METRICS_TX.inc_tx_with_status_and_type(
                    MetricsTxStatus::Failed,
                    MetricsTxType(head_tx.tx_type())
                ));
                txs.pop();
                continue;
            }
        };

        let account_diffs_in_tx = get_account_diffs_in_tx(&transaction_backup, context)?;
        let merged_diffs = merge_diffs(&account_diffs, account_diffs_in_tx);

        let (tx_size_without_accounts, new_accounts_diff_size) = calculate_tx_diff_size(
            &merged_diffs,
            &head_tx,
            &receipt,
            *DEPOSITS_LOG_LEN,
            *WITHDRAWAL_LOG_LEN,
        )?;

        if acc_size_without_accounts + tx_size_without_accounts + new_accounts_diff_size
            > SAFE_BYTES_PER_BLOB
        {
            debug!(
                "No more StateDiff space to run this transactions. Skipping transaction: {:?}",
                tx_hash
            );
            txs.pop();

            // This transaction is too big, we need to restore the state
            context.vm.restore_cache_state(transaction_backup)?;

            continue;
        }

        txs.shift()?;
        // Pull transaction from the mempool
        blockchain.remove_transaction_from_pool(&head_tx.tx.compute_hash())?;

        // We only add the withdrawals and deposits length because the accounts diffs may change
        acc_size_without_accounts += tx_size_without_accounts;
        size_accounts_diffs = new_accounts_diff_size;
        // Include the new accounts diffs
        account_diffs = merged_diffs;
        // Add transaction to block
        debug!("Adding transaction: {} to payload", tx_hash);
        context.payload.body.transactions.push(head_tx.into());
        // Save receipt for hash calculation
        context.receipts.push(receipt);
    }
    Ok(())
}

fn apply_transaction_l2(
    head: &HeadTransaction,
    context: &mut PayloadBuildContext,
) -> Result<(Receipt, CallFrameBackup), ChainError> {
    match **head {
        Transaction::EIP4844Transaction(_) => Err(ChainError::InvalidTransaction(
            "Blob transactions not supported in the L2".to_string(),
        )),
        _ => apply_plain_transaction_l2(head, context),
    }
}

fn apply_plain_transaction_l2(
    head: &HeadTransaction,
    context: &mut PayloadBuildContext,
) -> Result<(Receipt, CallFrameBackup), ChainError> {
    let (report, gas_used, transaction_backup) = context.vm.execute_tx_l2(
        &head.tx,
        &context.payload.header,
        &mut context.remaining_gas,
        head.tx.sender(),
    )?;
    context.block_value += U256::from(gas_used) * head.tip;
    Ok((report, transaction_backup))
}

/// Returns the state diffs introduced by the transaction by comparing the call frame backup
/// (which holds the state before executing the transaction) with the current state of the cache
/// (which contains all the writes performed by the transaction).
fn get_account_diffs_in_tx(
    call_frame_backup: &CallFrameBackup,
    context: &PayloadBuildContext,
) -> Result<HashMap<Address, AccountStateDiff>, BlockProducerError> {
    let mut modified_accounts = HashMap::new();
    match &context.vm {
        Evm::REVM { .. } => {
            return Err(BlockProducerError::EvmError(EvmError::InvalidEVM(
                "REVM not supported for L2".to_string(),
            )))
        }
        Evm::LEVM { db } => {
            // First we add the account info
            for (address, original_account) in call_frame_backup.original_accounts_info.iter() {
                let new_account =
                    db.cache
                        .get(address)
                        .ok_or(BlockProducerError::FailedToGetDataFrom(
                            "DB Cache".to_owned(),
                        ))?;

                let nonce_diff: u16 = (new_account.info.nonce - original_account.info.nonce)
                    .try_into()
                    .map_err(BlockProducerError::TryIntoError)?;

                let new_balance = if new_account.info.balance != original_account.info.balance {
                    Some(new_account.info.balance)
                } else {
                    None
                };

                let bytecode = if new_account.code != original_account.code {
                    Some(new_account.code.clone())
                } else {
                    None
                };

                let account_state_diff = AccountStateDiff {
                    new_balance,
                    nonce_diff,
                    storage: HashMap::new(), // We add the storage later
                    bytecode,
                    bytecode_hash: None,
                };

                modified_accounts.insert(*address, account_state_diff);
            }

            // Then if there is any storage change, we add it to the account state diff
            for (address, original_storage_slots) in
                call_frame_backup.original_account_storage_slots.iter()
            {
                let account_info =
                    db.cache
                        .get(address)
                        .ok_or(BlockProducerError::FailedToGetDataFrom(
                            "DB Cache".to_owned(),
                        ))?;

                let mut added_storage = HashMap::new();
                for key in original_storage_slots.keys() {
                    added_storage.insert(
                        *key,
                        *account_info.storage.get(key).ok_or(
                            BlockProducerError::FailedToGetDataFrom(
                                "Account info Storage".to_owned(),
                            ),
                        )?,
                    );
                }
                if let Some(account_state_diff) = modified_accounts.get_mut(address) {
                    account_state_diff.storage = added_storage;
                } else {
                    // If the account is not in the modified accounts, we create a new one
                    let account_state_diff = AccountStateDiff {
                        new_balance: None,
                        nonce_diff: 0,
                        storage: added_storage,
                        bytecode: None,
                        bytecode_hash: None,
                    };
                    modified_accounts.insert(*address, account_state_diff);
                }
            }
        }
    }
    Ok(modified_accounts)
}

/// Combines the diffs from the current transaction with the existing block diffs.
/// Transaction diffs represent state changes from the latest transaction execution,
/// while previous diffs accumulate all changes included in the block so far.
fn merge_diffs(
    previous_diffs: &HashMap<Address, AccountStateDiff>,
    tx_diffs: HashMap<Address, AccountStateDiff>,
) -> HashMap<Address, AccountStateDiff> {
    let mut merged_diffs = previous_diffs.clone();
    for (address, diff) in tx_diffs {
        if let Some(existing_diff) = merged_diffs.get_mut(&address) {
            // New balance could be None if a transaction didn't change the balance
            // but we want to keep the previous changes made in a transaction included in the block
            existing_diff.new_balance = diff.new_balance.or(existing_diff.new_balance);

            // We add the nonce diff to the existing one to keep track of the total nonce diff
            existing_diff.nonce_diff += diff.nonce_diff;

            // we need to overwrite only the new storage storage slot with the new values
            existing_diff.storage.extend(diff.storage);

            // Take the bytecode from the tx diff if present, avoiding clone if not needed
            if diff.bytecode.is_some() {
                existing_diff.bytecode = diff.bytecode;
            }

            // Take the new bytecode hash if it is present
            existing_diff.bytecode_hash = diff.bytecode_hash.or(existing_diff.bytecode_hash);
        } else {
            merged_diffs.insert(address, diff);
        }
    }
    merged_diffs
}

/// Calculates the size of the state diffs introduced by the transaction, including
/// the size of withdrawals and deposits logs for this transaction, and the total
/// size of all account diffs accumulated so far in the block.
/// This is necessary because each transaction can modify accounts that were already
/// changed by previous transactions, so we must recalculate the total diff size each time.
fn calculate_tx_diff_size(
    merged_diffs: &HashMap<Address, AccountStateDiff>,
    head_tx: &HeadTransaction,
    receipt: &Receipt,
    deposits_log_len: usize,
    withdrawals_log_len: usize,
) -> Result<(usize, usize), BlockProducerError> {
    let mut tx_state_diff_size = 0;
    let mut new_accounts_diff_size = 0;

    for (address, diff) in merged_diffs.iter() {
        let encoded = match diff.encode(address) {
            Ok(encoded) => encoded,
            Err(StateDiffError::EmptyAccountDiff) => {
                debug!("Skipping empty account diff for address: {address}");
                continue;
            }
            Err(e) => {
                error!("Failed to encode account state diff: {e}");
                return Err(BlockProducerError::FailedToEncodeAccountStateDiff(e));
            }
        };
        new_accounts_diff_size += encoded.len();
    }

    if is_deposit_l2(head_tx) {
        tx_state_diff_size += deposits_log_len;
    }
    if is_withdrawal_l2(&head_tx.clone().into(), receipt) {
        tx_state_diff_size += withdrawals_log_len;
    }

    Ok((tx_state_diff_size, new_accounts_diff_size))
}
