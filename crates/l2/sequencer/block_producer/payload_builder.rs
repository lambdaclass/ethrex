use std::sync::Arc;

use ethrex_blockchain::{
    constants::TX_GAS_COST,
    error::ChainError,
    payload::{PayloadBuildContext, PayloadBuildResult},
    Blockchain,
};
use ethrex_common::types::{Block, SAFE_BYTES_PER_BLOB};
use ethrex_metrics::metrics;

#[cfg(feature = "metrics")]
use ethrex_metrics::metrics_transactions::{MetricsTxStatus, MetricsTxType, METRICS_TX};
use ethrex_storage::Store;
use std::ops::Div;
use tokio::time::Instant;
use tracing::debug;

use crate::sequencer::{errors::BlockProducerError, state_diff::TX_STATE_DIFF_SIZE};

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

    debug!("Building payload on L2");
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
    // Two bytes for the len
    // 2 bytes for withdrawals + 2 bytes for deposits
    context.acc_state_diff_size = Some(4);

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
        let acc_state_diff_size = context
            .acc_state_diff_size
            .ok_or(BlockProducerError::Custom(
                "L2 should have access to accumulated state diff size".to_owned(),
            ))?;
        if acc_state_diff_size + TX_STATE_DIFF_SIZE > SAFE_BYTES_PER_BLOB {
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
        let receipt = match blockchain.apply_transaction(&head_tx, context) {
            Ok(receipt) => {
                txs.shift()?;
                // Pull transaction from the mempool
                blockchain.remove_transaction_from_pool(&head_tx.tx.compute_hash())?;

                metrics!(METRICS_TX.inc_tx_with_status_and_type(
                    MetricsTxStatus::Succeeded,
                    MetricsTxType(head_tx.tx_type())
                ));
                receipt
            }
            // This call is the part that differs from the original `fill_transactions`.
            Err(ChainError::EvmError(ethrex_vm::EvmError::StateDiffSizeError)) => {
                debug!(
                    "Skipping transaction: {}, doesn't fit in blob_size",
                    tx_hash
                );
                // We don't have enough space in the blob for the transaction, so we skip all txs from this account
                txs.pop();
                continue;
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
