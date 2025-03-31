use std::collections::HashMap;
use std::sync::Arc;

use ethrex_blockchain::{
    constants::TX_GAS_COST,
    payload::{PayloadBuildContext, PayloadBuildResult},
    Blockchain,
};
use ethrex_common::{
    types::{AccountInfo, Block, Receipt, Transaction, SAFE_BYTES_PER_BLOB},
    Address,
};
use ethrex_metrics::metrics;

#[cfg(feature = "metrics")]
use ethrex_metrics::metrics_transactions::{MetricsTxStatus, MetricsTxType, METRICS_TX};
use ethrex_storage::Store;
use std::ops::Div;
use tokio::time::Instant;
use tracing::debug;

use crate::{
    sequencer::{errors::BlockProducerError, state_diff::get_nonce_diff},
    utils::helpers::{is_deposit_l2, is_withdrawal_l2},
};

// transactions_root(H256) + receipts_root(H256) + gas_limit(u64) + gas_used(u64) + timestamp(u64) + base_fee_per_gas(u64).
// 32bytes + 32bytes + 8bytes + 8bytes + 8bytes + 8bytes
const HEADER_FIELDS_SIZE: usize = 96;

// address(H160) + amount(U256) + tx_hash(H256).
// 20bytes + 32bytes + 32bytes.
const L2_WITHDRAWAL_SIZE: usize = 84;

// address(H160) + amount(U256).
// 20bytes + 32bytes
const L2_DEPOSIT_SIZE: usize = 52;

/// L2 payload builder
/// Completes the payload building process, return the block value
/// Same as `blockchain::build_payload` without applying system operations and using a different `fill_transactions`
pub fn build_payload(
    blockchain: Arc<Blockchain>,
    payload: Block,
    store: &Store,
) -> Result<PayloadBuildResult, BlockProducerError> {
    let since = Instant::now();
    let gas_limit = payload.header.gas_limit;

    debug!("Building payload");
    let mut context = PayloadBuildContext::new(payload, blockchain.evm_engine, store)?;

    blockchain.apply_withdrawals(&mut context)?;
    fill_transactions(blockchain.clone(), &mut context, store)?;
    blockchain.extract_requests(&mut context)?;
    blockchain.finalize_payload(&mut context)?;

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
pub fn fill_transactions(
    blockchain: Arc<Blockchain>,
    context: &mut PayloadBuildContext,
    store: &Store,
) -> Result<(), BlockProducerError> {
    // Two bytes for the len
    let (mut acc_withdrawals_size, mut acc_deposits_size): (usize, usize) = (2, 2);
    let mut accounts_info_cache = HashMap::new();

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

        let previous_context = context.clone();

        // Execute tx
        let receipt = match blockchain.apply_transaction(&head_tx, context) {
            Ok(receipt) => {
                // This call is the part that differs from the original `fill_transactions`.
                if !check_state_diff_size(
                    &mut acc_withdrawals_size,
                    &mut acc_deposits_size,
                    head_tx.clone().into(),
                    &receipt,
                    context,
                    &mut accounts_info_cache,
                )? {
                    debug!(
                        "Skipping transaction: {}, doesn't feet in blob_size",
                        head_tx.tx.compute_hash()
                    );
                    // We don't have enough space in the blob for the transaction, so we skip all txs from this account
                    txs.pop();
                    *context = previous_context.clone();
                    continue;
                }
                txs.shift()?;
                // Pull transaction from the mempool
                blockchain.remove_transaction_from_pool(&head_tx.tx.compute_hash())?;

                metrics!(METRICS_TX.inc_tx_with_status_and_type(
                    MetricsTxStatus::Succeeded,
                    MetricsTxType(head_tx.tx_type())
                ));
                receipt
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
        // Add transaction to block
        debug!("Adding transaction: {} to payload", tx_hash);
        context.payload.body.transactions.push(head_tx.into());
        // Save receipt for hash calculation
        context.receipts.push(receipt);
    }
    Ok(())
}

/// Calculates the size of the current `StateDiff` of the block.
/// If the current size exceeds the blob size limit, returns `Ok(false)`.
/// If there is still space in the blob, returns `Ok(true)`.
/// StateDiff:
/// +-------------------+
/// | Version           |
/// | HeaderFields      |
/// | AccountsStateDiff |
/// | Withdrawals       |
/// | Deposits          |
/// +-------------------+
fn check_state_diff_size(
    acc_withdrawals_size: &mut usize,
    acc_deposits_size: &mut usize,
    tx: Transaction,
    receipt: &Receipt,
    context: &mut PayloadBuildContext,
    accounts_info_cache: &mut HashMap<Address, Option<AccountInfo>>,
) -> Result<bool, BlockProducerError> {
    if is_withdrawal_l2(&tx, receipt)? {
        *acc_withdrawals_size += L2_WITHDRAWAL_SIZE;
    }
    if is_deposit_l2(&tx) {
        *acc_deposits_size += L2_DEPOSIT_SIZE;
    }
    let modified_accounts_size = calc_modified_accounts_size(context, accounts_info_cache)?;

    let current_state_diff_size = 1 /* version (u8) */ + HEADER_FIELDS_SIZE + *acc_withdrawals_size + *acc_deposits_size + modified_accounts_size;

    if current_state_diff_size > SAFE_BYTES_PER_BLOB {
        // Restore the withdrawals and deposits counters.
        if is_withdrawal_l2(&tx, receipt)? {
            *acc_withdrawals_size -= L2_WITHDRAWAL_SIZE;
        }
        if is_deposit_l2(&tx) {
            *acc_deposits_size -= L2_DEPOSIT_SIZE;
        }
        debug!(
            "Blob size limit exceeded. current_state_diff_size: {}",
            current_state_diff_size
        );
        return Ok(false);
    }
    Ok(true)
}

fn calc_modified_accounts_size(
    context: &mut PayloadBuildContext,
    accounts_info_cache: &mut HashMap<Address, Option<AccountInfo>>,
) -> Result<usize, BlockProducerError> {
    let mut modified_accounts_size: usize = 2; // 2bytes | modified_accounts_len(u16)

    // We use a temporary_context because revm mutates it in `get_state_transitions`
    // TODO: remove when we stop using revm
    let mut temporary_context = context.clone();
    let account_updates = temporary_context
        .vm
        .get_state_transitions(context.payload.header.parent_hash)?;
    for account_update in account_updates {
        modified_accounts_size += 1 + 20; // 1byte + 20bytes | r#type(u8) + address(H160)
        if account_update.info.is_some() {
            modified_accounts_size += 32; // 32bytes | new_balance(U256)
        }

        let nonce_diff = get_nonce_diff(
            &account_update,
            &context.store,
            Some(accounts_info_cache),
            context.block_number(),
        )
        .map_err(|e| {
            BlockProducerError::Custom(format!("Block Producer failed to get nonce diff: {e}"))
        })?;
        if nonce_diff != 0 {
            modified_accounts_size += 2; // 2bytes | nonce_diff(u16)
        }
        // for each added_storage: 32bytes + 32bytes | key(H256) + value(U256)
        modified_accounts_size += account_update.added_storage.len() * 2 * 32;

        if let Some(bytecode) = &account_update.code {
            modified_accounts_size += 2; // 2bytes | bytecode_len(u16)
            modified_accounts_size += bytecode.len(); // (len)bytes | bytecode(Bytes)
        }
    }
    Ok(modified_accounts_size)
}
