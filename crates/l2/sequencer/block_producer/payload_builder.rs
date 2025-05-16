use std::collections::HashMap;
use std::sync::Arc;

use ethrex_blockchain::{
    constants::TX_GAS_COST,
    payload::{PayloadBuildContext, PayloadBuildResult},
    Blockchain,
};
use ethrex_common::{
    types::{Block, SAFE_BYTES_PER_BLOB},
    Address,
};
use ethrex_metrics::metrics;

#[cfg(feature = "metrics")]
use ethrex_metrics::metrics_transactions::{MetricsTxStatus, MetricsTxType, METRICS_TX};
use ethrex_storage::Store;
use ethrex_vm::backends::CallFrameBackup;
use std::ops::Div;
use tokio::time::Instant;
use tracing::{debug, error};

use crate::{
    sequencer::{
        errors::BlockProducerError,
        state_diff::{
            AccountStateDiff, L2_DEPOSIT_SIZE, L2_WITHDRAWAL_SIZE, LAST_HEADER_FIELDS_SIZE,
            TX_STATE_DIFF_SIZE,
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
    let mut size_without_accounts = 1 + LAST_HEADER_FIELDS_SIZE + 2 + 2 + 2;
    let mut size_accounts_diffs = 0;
    let mut actual_account_diffs = HashMap::new();

    let chain_config = store.get_chain_config()?;
    let max_blob_number_per_block: usize = chain_config
        .get_fork_blob_schedule(context.payload.header.timestamp)
        .map(|schedule| schedule.max)
        .unwrap_or_default()
        .try_into()
        .unwrap_or_default();

    debug!("Fetching transactions from mempool");
    // Fetch mempool transactions
    let (mut plain_txs, mut blob_txs) = blockchain.fetch_mempool_transactions(context)?;
    // Execute and add transactions to payload (if suitable)
    loop {
        // Check if we have enough gas to run more transactions
        if context.remaining_gas < TX_GAS_COST {
            debug!("No more gas to run transactions");
            break;
        };

        // Check if we have enough space for the StateDiff to run more transactions
        if size_without_accounts + size_accounts_diffs + TX_STATE_DIFF_SIZE > SAFE_BYTES_PER_BLOB {
            error!("No more StateDiff space to run transactions");
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

        // Check wether the tx is replay-protected
        if head_tx.tx.protected() && !chain_config.is_eip155_activated(context.block_number()) {
            // Ignore replay protected tx & all txs from the sender
            // Pull transaction from the mempool
            debug!("Ignoring replay-protected transaction: {}", tx_hash);
            txs.pop();
            blockchain.remove_transaction_from_pool(&head_tx.tx.compute_hash())?;
            continue;
        }

        // Increment the total transaction counter
        // CHECK: do we want it here to count every processed transaction
        // or we want it before the return?
        metrics!(METRICS_TX.inc_tx());

        // Execute tx
        let (receipt, call_frame_backup) = match blockchain.apply_transaction_l2(&head_tx, context)
        {
            Ok((receipt, call_frame_backup)) => {
                metrics!(METRICS_TX.inc_tx_with_status_and_type(
                    MetricsTxStatus::Succeeded,
                    MetricsTxType(head_tx.tx_type())
                ));
                (receipt, call_frame_backup)
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

        let account_diffs_actual_tx = get_tx_diffs(&call_frame_backup, context)?;
        let merged_diffs = merge_diffs(&actual_account_diffs, account_diffs_actual_tx);

        let mut tx_state_diff_size = 0;
        let mut new_accounts_diff_size = 0;

        for (address, diff) in merged_diffs.iter() {
            let (r, encoded) = diff
                .encode()
                .map_err(|_| BlockProducerError::Custom("CHANGE ERROR diff encode".to_owned()))?;
            new_accounts_diff_size += encoded.len();
            new_accounts_diff_size += r.to_be_bytes().len();
            new_accounts_diff_size += address.as_bytes().len();
        }

        if is_deposit_l2(&head_tx) {
            tx_state_diff_size += L2_DEPOSIT_SIZE;
        }
        if is_withdrawal_l2(&head_tx.clone().into(), &receipt)? {
            tx_state_diff_size += L2_WITHDRAWAL_SIZE;
        }

        if size_without_accounts + tx_state_diff_size + new_accounts_diff_size > SAFE_BYTES_PER_BLOB
        {
            error!(
                "No more StateDiff space to run transactions. Skipping transaction: {:?}",
                tx_hash
            );
            txs.pop();

            // REVERT DIFF IN VM
            context.vm.restore_cache_state(call_frame_backup)?;

            continue;
        }

        txs.shift()?;
        // Pull transaction from the mempool
        blockchain.remove_transaction_from_pool(&head_tx.tx.compute_hash())?;

        // We only add the withdrawals and deposits length because the accounts diffs may change
        size_without_accounts += tx_state_diff_size;
        size_accounts_diffs = new_accounts_diff_size;
        // Include the new accounts diffs
        actual_account_diffs = merged_diffs;
        // Add transaction to block
        debug!("Adding transaction: {} to payload", tx_hash);
        context.payload.body.transactions.push(head_tx.into());
        // Save receipt for hash calculation
        context.receipts.push(receipt);
    }
    Ok(())
}

fn get_tx_diffs(
    call_frame_backup: &CallFrameBackup,
    context: &PayloadBuildContext,
) -> Result<HashMap<Address, AccountStateDiff>, BlockProducerError> {
    let mut modified_accounts = HashMap::new();

    // get diffs
    // CHECK if indeed only the written accounts are backed up in this callframe backup
    match &context.vm {
        ethrex_vm::Evm::REVM { .. } => todo!(),
        ethrex_vm::Evm::LEVM { db } => {
            // First we add the account info
            for (address, original_account) in call_frame_backup.original_accounts_info.iter() {
                let new_account_info = db
                    .cache
                    .get(address)
                    .ok_or(BlockProducerError::StorageDataIsNone)?;

                let nonce_diff: u16 = (new_account_info.info.nonce - original_account.info.nonce)
                    .try_into()
                    .map_err(BlockProducerError::TryIntoError)?;

                let new_balance = if new_account_info.info.balance != original_account.info.balance
                {
                    Some(new_account_info.info.balance)
                } else {
                    None
                };

                let bytecode = if new_account_info.code != original_account.code {
                    Some(new_account_info.code.clone())
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
                let account_info = db
                    .cache
                    .get(address)
                    .ok_or(BlockProducerError::StorageDataIsNone)?;

                let mut added_storage = HashMap::new();
                // CHECK: if new slots are created, from zero to non-zero, are they here?
                for key in original_storage_slots.keys() {
                    added_storage.insert(
                        *key,
                        *account_info
                            .storage
                            .get(key)
                            .ok_or(BlockProducerError::StorageDataIsNone)?,
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

fn merge_diffs(
    previous_diffs: &HashMap<Address, AccountStateDiff>,
    new_diffs: HashMap<Address, AccountStateDiff>,
) -> HashMap<Address, AccountStateDiff> {
    let mut merged_diffs = previous_diffs.clone();
    for (address, diff) in new_diffs {
        if let Some(existing_diff) = merged_diffs.get_mut(&address) {
            existing_diff.new_balance = diff.new_balance;
            existing_diff.nonce_diff = diff.nonce_diff;
            // merge storage
            for (k, v) in diff.storage.iter() {
                existing_diff.storage.insert(*k, *v);
            }
            // Do we need to stay with the original bytecode if the new one is empty?
            existing_diff.bytecode = diff.bytecode;
            existing_diff.bytecode_hash = diff.bytecode_hash;
        } else {
            merged_diffs.insert(address, diff);
        }
    }
    merged_diffs
}
