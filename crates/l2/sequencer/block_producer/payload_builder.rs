use crate::sequencer::errors::BlockProducerError;
use ethrex_blockchain::{
    Blockchain,
    constants::TX_GAS_COST,
    payload::{PayloadBuildContext, PayloadBuildResult, TransactionQueue, apply_plain_transaction},
};
use ethrex_common::{
    U256,
    types::{Block, EIP1559_DEFAULT_SERIALIZED_LENGTH, SAFE_BYTES_PER_BLOB, Transaction, TxKind},
};
use ethrex_l2_common::{
    messages::get_block_l2_out_messages, privileged_transactions::PRIVILEGED_TX_BUDGET,
};
use ethrex_levm::vm::VMType;
use ethrex_metrics::metrics;
#[cfg(feature = "metrics")]
use ethrex_metrics::{
    blocks::METRICS_BLOCKS,
    transactions::{METRICS_TX, MetricsTxType},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;
use std::sync::Arc;
use std::{collections::HashMap, ops::Div};
use tokio::time::Instant;
use tracing::debug;

/// L2 payload builder
/// Completes the payload building process, return the block value
/// Same as `blockchain::build_payload` without applying system operations and using a different `fill_transactions`
pub async fn build_payload(
    blockchain: Arc<Blockchain>,
    payload: Block,
    store: &Store,
    privileged_nonces: &mut HashMap<u64, Option<u64>>,
    block_gas_limit: u64,
    registered_chains: Vec<U256>,
) -> Result<PayloadBuildResult, BlockProducerError> {
    let since = Instant::now();
    let gas_limit = payload.header.gas_limit;

    debug!("Building payload");
    let mut context = PayloadBuildContext::new(payload, store, &blockchain.options.r#type)?;

    fill_transactions(
        blockchain.clone(),
        &mut context,
        store,
        privileged_nonces,
        block_gas_limit,
        registered_chains,
    )
    .await?;
    blockchain.finalize_payload(&mut context)?;

    let interval = Instant::now().duration_since(since).as_millis();
    // TODO: expose as a proper metric
    tracing::info!("[METRIC] BUILDING PAYLOAD TOOK: {interval} ms");
    #[allow(clippy::as_conversions)]
    if let Some(gas_used) = gas_limit.checked_sub(context.remaining_gas) {
        let as_gigas = (gas_used as f64).div(10_f64.powf(9_f64));

        if interval != 0 {
            let throughput = (as_gigas) / (interval as f64) * 1000_f64;
            // TODO: expose as a proper metric
            tracing::info!(
                "[METRIC] BLOCK BUILDING THROUGHPUT: {throughput} Gigagas/s TIME SPENT: {interval} msecs"
            );
            metrics!(METRICS_BLOCKS.set_latest_gigagas(throughput));
        } else {
            metrics!(METRICS_BLOCKS.set_latest_gigagas(0_f64));
        }
    }

    metrics!(
        #[allow(clippy::as_conversions)]
        METRICS_BLOCKS.set_latest_block_gas_limit(gas_limit as f64);
        // L2 does not allow for blob transactions so the blob pool can be ignored
        let (tx_pool_size, _blob_pool_size) = blockchain
            .mempool
            .get_mempool_size()
            .inspect_err(|e| tracing::error!("Failed to get metrics for: mempool size {}", e.to_string()))
            .unwrap_or((0_u64, 0_u64));
        let _ = METRICS_TX
            .set_mempool_tx_count(tx_pool_size, false)
            .inspect_err(|e| tracing::error!("Failed to set metrics for: blob tx mempool size {}", e.to_string()));
    );

    Ok(context.into())
}

/// Same as `blockchain::fill_transactions` but enforces that the block encoded size
/// does not exceed `SAFE_BYTES_PER_BLOB`.
/// Also, uses a configured `block_gas_limit` to limit the gas used in the block,
/// which can be lower than the block gas limit specified in the payload header.
pub async fn fill_transactions(
    blockchain: Arc<Blockchain>,
    context: &mut PayloadBuildContext,
    store: &Store,
    privileged_nonces: &mut HashMap<u64, Option<u64>>,
    configured_block_gas_limit: u64,
    registered_chains: Vec<U256>,
) -> Result<(), BlockProducerError> {
    let mut privileged_tx_count = 0;
    let VMType::L2(fee_config) = context.vm.vm_type else {
        return Err(BlockProducerError::Custom("invalid VM type".to_string()));
    };
    let mut acc_encoded_size = context.payload.length();
    let fee_config_len = fee_config.to_vec().len();
    let chain_config = store.get_chain_config();
    let chain_id = chain_config.chain_id;

    debug!("Fetching transactions from mempool");
    // Fetch mempool transactions
    let latest_block_number = store.get_latest_block_number().await?;
    let mut txs = fetch_mempool_transactions(blockchain.as_ref(), context)?;

    // Execute and add transactions to payload (if suitable)
    loop {
        // Check if we have enough gas to run more transactions
        if context.remaining_gas < TX_GAS_COST {
            debug!("No more gas to run transactions");
            break;
        };

        // Check if we have enough gas to run more transactions within the configured block_gas_limit
        if context.gas_used() + TX_GAS_COST >= configured_block_gas_limit {
            debug!("No more gas to run transactions");
            break;
        }

        // Check if we have enough blob space to run more transactions
        if acc_encoded_size + fee_config_len + EIP1559_DEFAULT_SERIALIZED_LENGTH
            > SAFE_BYTES_PER_BLOB
        {
            debug!("No more blob space to run transactions");
            break;
        };

        // Fetch the next transaction
        let Some(head_tx) = txs.peek() else {
            break;
        };

        // Check if we have enough gas to run the transaction
        if context.remaining_gas < head_tx.tx.gas_limit() {
            debug!("Skipping transaction: {}, no gas left", head_tx.tx.hash());
            // We don't have enough gas left for the transaction, so we skip all txs from this account
            txs.pop();
            continue;
        }

        // Check if we have enough gas to run the transaction within the configured block_gas_limit
        if context.gas_used() + head_tx.tx.gas_limit() >= configured_block_gas_limit {
            debug!("Skipping transaction: {}, no gas left", head_tx.tx.hash());
            // We don't have enough gas left for the transaction, so we skip all txs from this account
            txs.pop();
            continue;
        }

        // Check if we have enough blob space to add this transaction
        let tx_ref: &Transaction = head_tx.tx.transaction();
        let tx_size = tx_ref.length();
        if acc_encoded_size + fee_config_len + tx_size > SAFE_BYTES_PER_BLOB {
            debug!("No more blob space to run transactions");
            break;
        };

        if let Transaction::PrivilegedL2Transaction(privileged_tx) = tx_ref {
            if privileged_tx_count >= PRIVILEGED_TX_BUDGET {
                debug!("Ran out of space for privileged transactions");
                txs.pop();
                continue;
            }
            let id = head_tx.nonce();
            let entry = privileged_nonces
                .entry(privileged_tx.chain_id)
                .or_insert(None);
            if (*entry).is_some_and(|last_nonce| id != last_nonce + 1) {
                debug!("Ignoring out-of-order privileged transaction");
                txs.pop();
                continue;
            }
        }

        // TODO: maybe fetch hash too when filtering mempool so we don't have to compute it here (we can do this in the same refactor as adding timestamp)
        let tx_hash = head_tx.tx.hash();

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

        if maybe_sender_acc_info.is_some_and(|acc_info| head_tx.nonce() < acc_info.nonce)
            && !head_tx.is_privileged()
        {
            debug!("Removing transaction with nonce too low from mempool: {tx_hash:#x}");
            txs.pop();
            blockchain.remove_transaction_from_pool(&tx_hash)?;
            continue;
        }

        // Set BAL index for this transaction (1-indexed per EIP-7928)
        #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
        let tx_index = (context.payload.body.transactions.len() + 1) as u16;
        context.vm.set_bal_index(tx_index);

        // Record tx sender and recipient for BAL
        if let Some(recorder) = context.vm.db.bal_recorder_mut() {
            recorder.record_touched_address(head_tx.tx.sender());
            if let TxKind::Call(to) = head_tx.to() {
                recorder.record_touched_address(to);
            }
        }

        // Execute tx
        let previous_remaining_gas = context.remaining_gas;
        let previous_block_value = context.block_value;
        let previous_cumulative_gas_spent = context.cumulative_gas_spent;
        let receipt = match apply_plain_transaction(&head_tx, context) {
            Ok(receipt) => receipt,
            Err(e) => {
                debug!("Failed to execute transaction: {}, {e}", tx_hash);
                metrics!(METRICS_TX.inc_tx_errors(e.to_metric()));
                // Ignore following txs from sender
                txs.pop();
                continue;
            }
        };

        let l2_messages = get_block_l2_out_messages(std::slice::from_ref(&receipt), chain_id);
        let mut found_invalid_message = false;
        for msg in l2_messages {
            if !registered_chains.contains(&msg.dest_chain_id) {
                txs.pop();
                context.vm.undo_last_tx()?;
                context.remaining_gas = previous_remaining_gas;
                context.block_value = previous_block_value;
                context.cumulative_gas_spent = previous_cumulative_gas_spent;
                found_invalid_message = true;
                break;
            }
        }
        if found_invalid_message {
            continue;
        }

        if let Transaction::PrivilegedL2Transaction(privileged_tx) = tx_ref {
            let id = head_tx.nonce();
            privileged_nonces.insert(privileged_tx.chain_id, Some(id));

            privileged_tx_count += 1;
        }

        // Update acc_encoded_size
        acc_encoded_size += tx_size;

        txs.shift()?;
        // Pull transaction from the mempool
        blockchain.remove_transaction_from_pool(&head_tx.tx.hash())?;

        // Add transaction to block
        context.payload.body.transactions.push(head_tx.into());

        // Save receipt for hash calculation
        context.receipts.push(receipt);
    } // end loop

    metrics!(
        context
            .payload
            .body
            .transactions
            .iter()
            .for_each(|tx| METRICS_TX.inc_tx_with_type(MetricsTxType(tx.tx_type())))
    );

    Ok(())
}

// TODO: Once #2857 is implemented, we can completely ignore the blobs pool.
fn fetch_mempool_transactions(
    blockchain: &Blockchain,
    context: &mut PayloadBuildContext,
) -> Result<TransactionQueue, BlockProducerError> {
    let (plain_txs, mut blob_txs) = blockchain.fetch_mempool_transactions(context)?;
    while let Some(blob_tx) = blob_txs.peek() {
        let tx_hash = blob_tx.hash();
        blockchain.remove_transaction_from_pool(&tx_hash)?;
        blob_txs.pop();
    }
    Ok(plain_txs)
}
