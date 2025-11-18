use crate::sequencer::{
    errors::BlockProducerError,
    l1_committer::{self, L1Committer},
};
use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, SuperBlockchain,
    constants::{POST_OSAKA_GAS_LIMIT_CAP, TX_GAS_COST},
    payload::{
        HeadTransaction, PayloadBuildContext, PayloadBuildResult, TransactionQueue,
        apply_plain_transaction,
    },
    vm::StoreVmDatabase,
};
use ethrex_common::{
    Address, H256, Signature, U256,
    types::{
        Block, BlockHeader, EIP1559Transaction, GenericTransaction, MempoolTransaction,
        PrivilegedL2Transaction, Receipt, SAFE_BYTES_PER_BLOB, Transaction, TxKind, TxType,
        account_diff::{AccountStateDiff, get_accounts_diff_size},
    },
    utils::keccak,
};
use ethrex_l2_common::state_diff::{
    BLOCK_HEADER_LEN, L1MESSAGE_LOG_LEN, PRIVILEGED_TX_LOG_LEN, SIMPLE_TX_STATE_DIFF_SIZE,
};
use ethrex_l2_common::{
    l1_messages::get_block_l1_messages, privileged_transactions::PRIVILEGED_TX_BUDGET,
};
use ethrex_levm::{hooks::l2_hook::COMMON_BRIDGE_L2_ADDRESS, utils::get_account_diffs_in_tx};
use ethrex_metrics::metrics;
#[cfg(feature = "metrics")]
use ethrex_metrics::{
    metrics_blocks::METRICS_BLOCKS,
    metrics_transactions::{METRICS_TX, MetricsTxType},
};
use ethrex_rlp::encode::PayloadRLPEncode;
use ethrex_storage::Store;
use ethrex_vm::{EvmError, ExecutionResult};
use secp256k1::{Message, SECP256K1, SecretKey};
use spawned_concurrency::tasks::GenServerHandle;
use std::ops::Div;
use std::sync::Arc;
use std::{collections::HashMap, str::FromStr};
use tokio::time::Instant;
use tracing::{debug, error, info};

/// L2 payload builder
/// Completes the payload building process, return the block value
/// Same as `blockchain::build_payload` without applying system operations and using a different `fill_transactions`
pub async fn build_payload(
    super_blockchain: Arc<SuperBlockchain>,
    payload: Block,
    store: &Store,
    last_privileged_nonce: &mut Option<u64>,
    block_gas_limit: u64,
) -> Result<PayloadBuildResult, BlockProducerError> {
    let since = Instant::now();
    let gas_limit = payload.header.gas_limit;

    debug!("Building payload");
    let mut context = PayloadBuildContext::new(
        payload,
        store,
        &super_blockchain.main_blockchain.options.r#type,
    )?;

    fill_transactions(
        super_blockchain.clone(),
        &mut context,
        store,
        last_privileged_nonce,
        block_gas_limit,
    )
    .await?;
    super_blockchain
        .main_blockchain
        .finalize_payload(&mut context)?;

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
        let (tx_pool_size, _blob_pool_size) = super_blockchain
            .main_blockchain
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

/// Same as `blockchain::fill_transactions` but enforces that the `StateDiff` size
/// stays within the blob size limit after processing each transaction.
/// Also, uses a configured `block_gas_limit` to limit the gas used in the block,
/// which can be lower than the block gas limit specified in the payload header.
pub async fn fill_transactions(
    super_blockchain: Arc<SuperBlockchain>,
    context: &mut PayloadBuildContext,
    store: &Store,
    last_privileged_nonce: &mut Option<u64>,
    configured_block_gas_limit: u64,
) -> Result<(), BlockProducerError> {
    // version (u8) + header fields (struct) + messages_len (u16) + privileged_tx_len (u16) + accounts_diffs_len (u16)
    let mut acc_size_without_accounts = 1 + BLOCK_HEADER_LEN + 2 + 2 + 2;
    let mut size_accounts_diffs = 0;
    let mut account_diffs = HashMap::new();
    let safe_bytes_per_blob: u64 = SAFE_BYTES_PER_BLOB.try_into()?;
    let mut privileged_tx_count = 0;

    let chain_config = store.get_chain_config();

    debug!("Fetching transactions from mempool");
    // Fetch mempool transactions
    let latest_block_number = store.get_latest_block_number().await?;
    let mut txs = fetch_mempool_transactions(&super_blockchain.main_blockchain.as_ref(), context)?;

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

        // Check if we have enough space for the StateDiff to run more transactions
        if acc_size_without_accounts + size_accounts_diffs + SIMPLE_TX_STATE_DIFF_SIZE
            > safe_bytes_per_blob
        {
            debug!("No more StateDiff space to run transactions");
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

        // TODO: maybe fetch hash too when filtering mempool so we don't have to compute it here (we can do this in the same refactor as adding timestamp)
        let tx_hash = head_tx.tx.hash();

        // Check whether the tx is replay-protected
        if head_tx.tx.protected() && !chain_config.is_eip155_activated(context.block_number()) {
            // Ignore replay protected tx & all txs from the sender
            // Pull transaction from the mempool
            debug!("Ignoring replay-protected transaction: {}", tx_hash);
            txs.pop();
            super_blockchain
                .main_blockchain
                .remove_transaction_from_pool(&tx_hash)?;
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
            super_blockchain
                .main_blockchain
                .remove_transaction_from_pool(&tx_hash)?;
            continue;
        }

        // Copy remaining gas and block value before executing the transaction
        let previous_remaining_gas = context.remaining_gas;
        let previous_block_value = context.block_value;

        // Skip simulating calls to the CommonBridgeL2 contract
        // We only want to simulate transactions that would require L1 simulation,
        // and this is not the case for CommonBridge calls
        //
        // CAUTION: not skipping simulating these transactions would lead to
        // a simulation failure in some cases.
        if head_tx.to() != TxKind::Call(COMMON_BRIDGE_L2_ADDRESS) {
            let mut sub_context = PayloadBuildContext::new(
                context.payload.clone(),
                &context.store,
                &super_blockchain.main_blockchain.options.r#type,
            )?;

            let block_header = super_blockchain
                .main_blockchain
                .storage
                .get_block_header_by_hash(context.parent_hash())
                .inspect_err(|e| error!("{e}"))?
                .unwrap();

            let sim = match sub_context
                .vm
                .simulate_tx_from_generic(&head_tx.tx.clone().into(), &block_header)
            {
                Ok(sim_result) => sim_result,
                Err(e) => {
                    error!(from =% head_tx.tx.clone().sender(), to =? head_tx.tx.clone().to(), data = hex::encode(head_tx.tx.clone().data()), "{e}");
                    println!("[L2 Builder] Head transaction simulation failed ({tx_hash:#x}): {e}");
                    continue;
                }
            };

            for log in sim.logs() {
                if log.address == COMMON_BRIDGE_L2_ADDRESS
                    && log.topics.contains(
                        &H256::from_str(
                            "b0e76942d2929d9dcf5c6b8e32bf27df13e118fcaab4cef2e90257551bba0270",
                        )
                        .unwrap(),
                    )
                {
                    println!("[L2 Builder] Detected call to CommonBridge");

                    let from = Address::from_slice(log.data.get(0x20 - 20..0x20).unwrap());
                    let to = Address::from_slice(log.data.get(0x40 - 20..0x40).unwrap());
                    let value = U256::from_big_endian(log.data.get(0x40..0x60).unwrap());
                    let data_len =
                        U256::from_big_endian(log.data.get(0x80..0xa0).unwrap()).as_usize();
                    let data = &log.data.iter().as_slice()[0xa0..0xa0 + data_len];

                    info!(%from, %to, ?value, data = hex::encode(data), "Executing call on L1");
                    let l1 = super_blockchain.secondary_blockchain.as_ref().unwrap();

                    let transaction = GenericTransaction {
                        r#type: TxType::EIP1559,
                        to: TxKind::Call(to),
                        from,
                        value,
                        input: Bytes::copy_from_slice(data),
                        ..Default::default()
                    };

                    println!("[L2 Builder] Simulating transaction in L1");

                    let block_header = l1
                        .storage
                        .get_block_header(l1.storage.get_latest_block_number().await.unwrap())
                        .unwrap()
                        .unwrap();
                    let result =
                        simulate_tx(&transaction, &block_header, l1.storage.clone(), l1.clone())
                            .await
                            .inspect(|res| {
                                info!(
                                    success = res.is_success(),
                                    "RESULT: {}",
                                    hex::encode(res.output())
                                )
                            })
                            .inspect_err(|e| {
                                error!("SIMULATE ERROR: {e}");
                                println!("[L2 Builder] L1 Simulation failed: {e}");
                            })?;

                    // 0x57272f8e
                    // keccak(to || data)
                    // 0x40
                    // response_length
                    // response || padding
                    let response_len = result.output().len();
                    let padding = response_len % 32;

                    let data = [
                        &[0x57, 0x27, 0x2f, 0x8e],
                        keccak([to.as_bytes(), data].concat()).as_bytes(),
                        H256::from_str(
                            "0x0000000000000000000000000000000000000000000000000000000000000040",
                        )
                        .unwrap()
                        .as_bytes(),
                        U256::from_big_endian(&response_len.to_be_bytes())
                            .to_big_endian()
                            .as_slice(),
                        result.output().iter().as_slice(),
                        &vec![0; padding],
                    ]
                    .concat();

                    let from =
                        Address::from_str("000000000000000000000000000000000000fff0").unwrap();

                    let tx = PrivilegedL2Transaction {
                        chain_id: super_blockchain
                            .main_blockchain
                            .storage
                            .chain_config
                            .chain_id,
                        nonce: context
                            .store
                            .get_nonce_by_account_address(
                                context.store.get_latest_block_number().await.unwrap(),
                                from,
                            )
                            .await
                            .unwrap()
                            .unwrap(),
                        from,
                        max_priority_fee_per_gas: 1000000000000,
                        max_fee_per_gas: 1000000000000,
                        gas_limit: POST_OSAKA_GAS_LIMIT_CAP - 1,
                        to: TxKind::Call(COMMON_BRIDGE_L2_ADDRESS),
                        value: U256::zero(),
                        data: data.into(),
                        access_list: vec![],
                        inner_hash: Default::default(),
                    };

                    let mempool_tx =
                        MempoolTransaction::new(Transaction::PrivilegedL2Transaction(tx), from);
                    let head_tx = HeadTransaction {
                        tx: mempool_tx,
                        tip: 0,
                    };

                    println!(
                        "[L2 Builder] Presetting L1 response: {:#x}",
                        head_tx.tx.hash()
                    );

                    let receipt = match super_blockchain
                        .main_blockchain
                        .apply_transaction(&head_tx, context)
                    {
                        Ok(receipt) => {
                            println!(
                                "[L2 Builder] L1 response preset successfully ({:#x})",
                                head_tx.tx.hash()
                            );
                            receipt
                        }
                        Err(e) => {
                            error!("ERROR: {e}");
                            panic!(
                                "[L2 Builder] Failed to preset L1 response ({:#x}): {e}",
                                head_tx.tx.hash()
                            );
                        }
                    };

                    let tx_backup = context.vm.db.get_tx_backup().map_err(|e| {
                        BlockProducerError::FailedToGetDataFrom(format!("transaction backup: {e}"))
                    })?;
                    let account_diffs_in_tx = get_account_diffs_in_tx(&context.vm.db, tx_backup)
                        .map_err(|e| {
                            BlockProducerError::Custom(format!(
                                "Failed to get account diffs from tx: {e}"
                            ))
                        })?;
                    let merged_diffs = merge_diffs(&account_diffs, account_diffs_in_tx);

                    let (tx_size_without_accounts, new_accounts_diff_size) =
                        calculate_tx_diff_size(&merged_diffs, &head_tx, &receipt)?;

                    if acc_size_without_accounts + tx_size_without_accounts + new_accounts_diff_size
                        > safe_bytes_per_blob
                    {
                        debug!(
                            "No more StateDiff space to run this transactions. Skipping transaction: {:?}",
                            tx_hash
                        );
                        txs.pop();

                        // This transaction state change is too big, we need to undo it.
                        undo_last_tx(context, previous_remaining_gas, previous_block_value)?;
                        continue;
                    }

                    // Check we don't have an excessive number of privileged transactions
                    if head_tx.tx_type() == TxType::Privileged {
                        if privileged_tx_count >= PRIVILEGED_TX_BUDGET {
                            debug!("Ran out of space for privileged transactions");
                            txs.pop();
                            undo_last_tx(context, previous_remaining_gas, previous_block_value)?;
                            continue;
                        }
                        let id = head_tx.nonce();
                        if last_privileged_nonce.is_some_and(|last_nonce| id != last_nonce + 1) {
                            debug!("Ignoring out-of-order privileged transaction");
                            txs.pop();
                            undo_last_tx(context, previous_remaining_gas, previous_block_value)?;
                            continue;
                        }
                        last_privileged_nonce.replace(id);
                        privileged_tx_count += 1;
                    }

                    // We only add the messages and privileged transaction length because the accounts diffs may change
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
            }
        }

        // Execute tx
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

        let tx_backup = context.vm.db.get_tx_backup().map_err(|e| {
            BlockProducerError::FailedToGetDataFrom(format!("transaction backup: {e}"))
        })?;
        let account_diffs_in_tx =
            get_account_diffs_in_tx(&context.vm.db, tx_backup).map_err(|e| {
                BlockProducerError::Custom(format!("Failed to get account diffs from tx: {e}"))
            })?;
        let merged_diffs = merge_diffs(&account_diffs, account_diffs_in_tx);

        let (tx_size_without_accounts, new_accounts_diff_size) =
            calculate_tx_diff_size(&merged_diffs, &head_tx, &receipt)?;

        if acc_size_without_accounts + tx_size_without_accounts + new_accounts_diff_size
            > safe_bytes_per_blob
        {
            debug!(
                "No more StateDiff space to run this transactions. Skipping transaction: {:?}",
                tx_hash
            );
            txs.pop();

            // This transaction state change is too big, we need to undo it.
            undo_last_tx(context, previous_remaining_gas, previous_block_value)?;
            continue;
        }

        // Check we don't have an excessive number of privileged transactions
        if head_tx.tx_type() == TxType::Privileged {
            if privileged_tx_count >= PRIVILEGED_TX_BUDGET {
                debug!("Ran out of space for privileged transactions");
                txs.pop();
                undo_last_tx(context, previous_remaining_gas, previous_block_value)?;
                continue;
            }
            let id = head_tx.nonce();
            if last_privileged_nonce.is_some_and(|last_nonce| id != last_nonce + 1) {
                debug!("Ignoring out-of-order privileged transaction");
                txs.pop();
                undo_last_tx(context, previous_remaining_gas, previous_block_value)?;
                continue;
            }
            last_privileged_nonce.replace(id);
            privileged_tx_count += 1;
        }

        txs.shift()?;
        // Pull transaction from the mempool
        super_blockchain
            .main_blockchain
            .remove_transaction_from_pool(&head_tx.tx.hash())?;

        // We only add the messages and privileged transaction length because the accounts diffs may change
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
/// the size of messages and privileged transactions, and the total
/// size of all account diffs accumulated so far in the block.
/// This is necessary because each transaction can modify accounts that were already
/// changed by previous transactions, so we must recalculate the total diff size each time.
fn calculate_tx_diff_size(
    merged_diffs: &HashMap<Address, AccountStateDiff>,
    head_tx: &HeadTransaction,
    receipt: &Receipt,
) -> Result<(u64, u64), BlockProducerError> {
    let new_accounts_diff_size = get_accounts_diff_size(merged_diffs).map_err(|e| {
        BlockProducerError::Custom(format!("Failed to calculate account diffs size: {}", e))
    })?;

    let mut tx_state_diff_size = 0;

    if is_privileged_tx(head_tx) {
        tx_state_diff_size += PRIVILEGED_TX_LOG_LEN;
    }
    let l1_message_count: u64 = get_block_l1_messages(std::slice::from_ref(receipt))
        .len()
        .try_into()?;
    tx_state_diff_size += l1_message_count * L1MESSAGE_LOG_LEN;

    Ok((tx_state_diff_size, new_accounts_diff_size))
}

fn is_privileged_tx(tx: &Transaction) -> bool {
    matches!(tx, Transaction::PrivilegedL2Transaction(_tx))
}

fn undo_last_tx(
    context: &mut PayloadBuildContext,
    previous_remaining_gas: u64,
    previous_block_value: U256,
) -> Result<(), BlockProducerError> {
    context.vm.undo_last_tx()?;
    context.remaining_gas = previous_remaining_gas;
    context.block_value = previous_block_value;
    Ok(())
}

async fn simulate_tx(
    transaction: &GenericTransaction,
    block_header: &BlockHeader,
    storage: Store,
    blockchain: Arc<Blockchain>,
) -> Result<ExecutionResult, EvmError> {
    let vm_db = StoreVmDatabase::new(storage, block_header.clone());
    let mut vm = blockchain.new_evm(vm_db)?;

    vm.simulate_tx_from_generic(transaction, block_header)
}
