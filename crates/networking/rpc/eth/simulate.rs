use std::collections::HashMap;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    types::{
        block_identifier::{BlockIdentifier, BlockIdentifierOrHash},
        simulate::{
            AccountOverride, BlockOverrides, CallError, CallResult, SimulatePayload,
            SimulatedBlock, SimulatedLog, SimulatedTransaction,
        },
    },
    utils::RpcErr,
};
use ethrex_blockchain::{overlay_vm_db::OverlayVmDatabase, vm::StoreVmDatabase};
use ethrex_common::{
    Address, Bytes, H256, U256,
    constants::{DEFAULT_REQUESTS_HASH, EMPTY_WITHDRAWALS_HASH},
    types::{
        AccessListEntry, Block, BlockBody, BlockHeader, GenericTransaction, Log, Receipt, TxKind,
        TxType, bloom_from_logs, compute_receipts_root, compute_transactions_root,
        transaction::{
            EIP1559Transaction, EIP2930Transaction, EIP4844Transaction, EIP7702Transaction,
            LegacyTransaction, Transaction,
        },
    },
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::Trie;
use ethrex_vm::{ExecutionResult, backends::Evm};
use serde_json::Value;
use tracing::debug;

const MAX_BLOCK_STATE_CALLS: usize = 256;

/// The special address used for synthetic traceTransfers logs.
/// 0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee
fn trace_transfer_address() -> Address {
    Address::from([0xee_u8; 20])
}

/// ERC-20 Transfer event topic:
/// keccak256("Transfer(address,address,uint256)")
/// = 0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef
const TRANSFER_EVENT_SIGNATURE: H256 = H256([
    0xdd, 0xf2, 0x52, 0xad, 0x1b, 0xe2, 0xc8, 0x9b, 0x69, 0xc2, 0xb0, 0x68, 0xfc, 0x37, 0x8d, 0xaa,
    0x95, 0x2b, 0xa7, 0xf1, 0x63, 0xc4, 0xa1, 0x16, 0x28, 0xf5, 0x5a, 0x4d, 0xf5, 0x23, 0xb3, 0xef,
]);

pub struct SimulateV1Request {
    pub payload: SimulatePayload,
    pub block: BlockIdentifierOrHash,
}

impl RpcHandler for SimulateV1Request {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.is_empty() {
            return Err(RpcErr::BadParams("No params provided".to_owned()));
        }
        if params.len() > 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected one or two params and {} were provided",
                params.len()
            )));
        }

        let payload: SimulatePayload = serde_json::from_value(params[0].clone())?;

        if payload.block_state_calls.len() > MAX_BLOCK_STATE_CALLS {
            return Err(RpcErr::BadParams(format!(
                "Too many block state calls: {} (max {})",
                payload.block_state_calls.len(),
                MAX_BLOCK_STATE_CALLS
            )));
        }

        // Validate mutually exclusive state/stateDiff.
        for bsc in &payload.block_state_calls {
            if let Some(overrides) = &bsc.state_overrides {
                for (addr, o) in overrides {
                    if o.state.is_some() && o.state_diff.is_some() {
                        return Err(RpcErr::BadParams(format!(
                            "Account {addr:?} has both state and stateDiff overrides"
                        )));
                    }
                }
            }
        }

        let block = match params.get(1) {
            Some(value) => BlockIdentifierOrHash::parse(value.clone(), 1)?,
            None => BlockIdentifierOrHash::Identifier(BlockIdentifier::default()),
        };

        Ok(Self { payload, block })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!("eth_simulateV1 on block: {}", self.block);

        // 1. Resolve base block header.
        let base_header = match self.block.resolve_block_header(&context.storage).await? {
            Some(header) => header,
            None => return Err(RpcErr::BadParams("Block not found".to_owned())),
        };

        // 2. Create base StoreVmDatabase.
        let base_vm_db = StoreVmDatabase::new(context.storage.clone(), base_header.clone())?;

        // 3. Initialize overlay.
        let mut overlay = OverlayVmDatabase::new(base_vm_db);
        let mut prev_header = base_header;
        let mut results: Vec<SimulatedBlock> = Vec::new();

        // State trie maintained across blocks for stateRoot computation.
        // Initialized lazily from the base block's state trie on first use.
        let mut state_trie: Option<Trie> = None;

        // 4. Iterate through block state calls.
        for block_state_call in self.payload.block_state_calls.iter() {
            // 4a. Build simulated block header.
            let mut sim_header =
                build_simulated_header(&prev_header, block_state_call.block_overrides.as_ref())?;

            // 4b. Validate block sequence (number must be strictly increasing).
            validate_block_sequence(&prev_header, &sim_header)?;

            // 4b'. Gap filling: emit empty intermediate blocks if block number jumps.
            let gap = sim_header.number - prev_header.number - 1;
            if gap > 0 {
                for i in 1..=gap {
                    let intermediate_number = prev_header.number + i;
                    let intermediate_timestamp = prev_header
                        .timestamp
                        .checked_add(i)
                        .ok_or_else(|| RpcErr::BadParams("Timestamp overflow".to_owned()))?;

                    let (empty_block, new_prev) = build_empty_intermediate_block(
                        &prev_header,
                        intermediate_number,
                        intermediate_timestamp,
                    );

                    // Register block hash in overlay for BLOCKHASH opcode.
                    overlay.set_block_hash(new_prev.number, empty_block.hash);

                    results.push(empty_block);
                    prev_header = new_prev;
                }

                // Fix sim_header's parent_hash to point to the last intermediate block.
                sim_header.parent_hash = prev_header.hash();
                // Reset cached hash since parent_hash changed.
                sim_header.hash = Default::default();

                // After gap filling, sim_header.timestamp may be stale (it was set
                // relative to the original prev, not the last gap block).
                if sim_header.timestamp <= prev_header.timestamp {
                    let had_explicit_time = block_state_call
                        .block_overrides
                        .as_ref()
                        .and_then(|o| o.time)
                        .is_some();
                    if had_explicit_time {
                        return Err(RpcErr::SimulateError {
                            code: -38021,
                            message: format!(
                                "block timestamps must be in order: {} <= {}",
                                sim_header.timestamp, prev_header.timestamp
                            ),
                        });
                    }
                    // No explicit override: auto-advance past the last gap block.
                    sim_header.timestamp = prev_header.timestamp + 1;
                    sim_header.hash = Default::default();
                }
            }

            // 4c. Apply state overrides for this block.
            if let Some(state_overrides) = &block_state_call.state_overrides {
                apply_state_overrides(&mut overlay, state_overrides);
            }

            // 4d. Create EVM for this block (clone overlay so it stays clean for extraction).
            let mut evm = Evm::new_for_l1(overlay.clone());

            // Get chain config for gas limit computation and chain_id.
            let chain_config = overlay
                .get_chain_config()
                .map_err(|e| RpcErr::Internal(e.to_string()))?;
            let chain_id = chain_config.chain_id;
            let fork = chain_config.get_fork(sim_header.timestamp);

            // 4e. Execute each call.
            let mut call_results = Vec::new();
            let mut block_gas_used: u64 = 0;
            let mut cumulative_log_count: u64 = 0;
            let blob_base_fee_override = block_state_call
                .block_overrides
                .as_ref()
                .and_then(|o| o.blob_base_fee);

            // Collect (Transaction, tx_hash, effective_gas_limit, nonce, from) tuples for response
            // construction. These are built after execution so gas defaults are known.
            struct TxInfo {
                tx: Transaction,
                hash: H256,
                from: Address,
                gas_limit: u64,
                nonce: u64,
                gas_price: u64,
            }
            let mut tx_infos: Vec<TxInfo> = Vec::new();

            for (tx_idx, generic_tx) in block_state_call.calls.iter().enumerate() {
                // Auto-fill nonce from current EVM state when not provided.
                let mut tx_with_nonce = generic_tx.clone();
                if tx_with_nonce.nonce.is_none() {
                    let sender_nonce = evm
                        .db
                        .get_account(generic_tx.from)
                        .map(|acc| acc.info.nonce)
                        .unwrap_or(0);
                    tx_with_nonce.nonce = Some(sender_nonce);
                }

                // Compute the gas limit this tx will use (replicating VM logic).
                // In validation mode, gas is capped by remaining block gas.
                let max_gas = ethrex_vm::backends::levm::get_max_allowed_gas_limit(
                    sim_header.gas_limit,
                    fork,
                );
                let tx_gas_limit = if let Some(g) = tx_with_nonce.gas {
                    g
                } else {
                    // In validation mode, remaining gas constrains the tx gas.
                    if self.payload.validation {
                        max_gas.saturating_sub(block_gas_used)
                    } else {
                        max_gas
                    }
                };

                let exec_result = evm.simulate_tx_from_generic_with_validation(
                    &tx_with_nonce,
                    &sim_header,
                    self.payload.validation,
                    blob_base_fee_override,
                );

                // When validation is enabled, VM errors are top-level errors.
                if self.payload.validation
                    && let Err(ref err) = exec_result
                {
                    return Err(map_vm_error_to_simulate_error(err));
                }

                let succeeded = matches!(exec_result, Ok(ExecutionResult::Success { .. }));
                let call_result = execution_result_to_call_result(
                    exec_result,
                    &sim_header,
                    cumulative_log_count,
                    H256::zero(), // placeholder for block_hash
                    H256::zero(), // placeholder for tx_hash
                    tx_idx as u64,
                );
                block_gas_used += call_result.gas_used;
                cumulative_log_count += call_result.logs.len() as u64;
                call_results.push(call_result);

                // Build unsigned Transaction for hashing and response.
                let nonce = tx_with_nonce.nonce.unwrap_or_default();
                let unsigned_tx =
                    generic_tx_to_unsigned_transaction(&tx_with_nonce, tx_gas_limit, chain_id);
                let tx_hash = unsigned_tx.hash();

                // Effective gas price: min(max_fee_per_gas, base_fee + max_priority_fee)
                let base_fee = sim_header.base_fee_per_gas.unwrap_or(0);
                let effective_gas_price = compute_effective_gas_price(generic_tx, base_fee);

                tx_infos.push(TxInfo {
                    tx: unsigned_tx,
                    hash: tx_hash,
                    from: generic_tx.from,
                    gas_limit: tx_gas_limit,
                    nonce,
                    gas_price: effective_gas_price,
                });

                // Inject traceTransfers synthetic log for top-level ETH value transfer.
                if self.payload.trace_transfers
                    && succeeded
                    && !generic_tx.value.is_zero()
                    && let TxKind::Call(to_addr) = generic_tx.to
                    && generic_tx.from != to_addr
                {
                    let last = call_results
                        .last_mut()
                        .expect("call_results should have at least one entry");
                    let transfer_log = create_trace_transfer_log(
                        generic_tx.from,
                        to_addr,
                        generic_tx.value,
                        cumulative_log_count - last.logs.len() as u64,
                        sim_header.number,
                        sim_header.timestamp,
                        H256::zero(), // block_hash placeholder
                        tx_hash,
                        tx_idx as u64,
                    );
                    last.logs.insert(0, transfer_log);
                    cumulative_log_count += 1;
                    // Re-index the existing logs after the inserted one.
                    let base_idx = cumulative_log_count - last.logs.len() as u64;
                    for (i, log) in last.logs.iter_mut().enumerate() {
                        log.log_index = base_idx + i as u64;
                    }
                }
            }

            // 4f. Process withdrawals if specified in block overrides.
            let withdrawals = block_state_call
                .block_overrides
                .as_ref()
                .and_then(|o| o.withdrawals.clone())
                .unwrap_or_default();
            if !withdrawals.is_empty() {
                evm.process_withdrawals(&withdrawals)?;
            }

            // 4g. Extract state transitions and merge into overlay.
            let account_updates = evm.get_state_transitions()?;
            overlay.merge_account_updates(&account_updates);

            // 4h. Build the final header with computed fields.
            let mut final_header = sim_header.clone();
            final_header.gas_used = block_gas_used;
            // Reset cached hash since gas_used changed.
            final_header.hash = Default::default();

            // Compute transactionsRoot from unsigned transactions.
            let transactions: Vec<Transaction> = tx_infos.iter().map(|ti| ti.tx.clone()).collect();
            final_header.transactions_root = compute_transactions_root(&transactions);

            // Compute receiptsRoot using EVM-only logs (not synthetic traceTransfers).
            // The receipts use the "real" logs only.
            let mut cumulative_gas = 0u64;
            let receipts: Vec<Receipt> = call_results
                .iter()
                .zip(block_state_call.calls.iter())
                .map(|(cr, generic_tx)| {
                    cumulative_gas += cr.gas_used;
                    // Filter out synthetic traceTransfers logs (address == 0xee...ee).
                    let real_logs: Vec<Log> = cr
                        .logs
                        .iter()
                        .filter(|l| l.address != trace_transfer_address())
                        .map(|sl| Log {
                            address: sl.address,
                            topics: sl.topics.clone(),
                            data: sl.data.clone(),
                        })
                        .collect();
                    Receipt::new(generic_tx.r#type, cr.status == 1, cumulative_gas, real_logs)
                })
                .collect();
            final_header.receipts_root = compute_receipts_root(&receipts);

            // Compute logsBloom from EVM-only logs (exclude synthetic traceTransfers).
            let all_real_logs: Vec<Log> = call_results
                .iter()
                .flat_map(|cr| {
                    cr.logs
                        .iter()
                        .filter(|l| l.address != trace_transfer_address())
                        .map(|sl| Log {
                            address: sl.address,
                            topics: sl.topics.clone(),
                            data: sl.data.clone(),
                        })
                })
                .collect();
            final_header.logs_bloom = bloom_from_logs(&all_real_logs);

            // Compute stateRoot using the persistent state trie.
            let base_block_hash = overlay.base_block_hash();
            let store = overlay.store().clone();
            let trie = if let Some(ref mut t) = state_trie {
                t
            } else {
                let initial_trie = store
                    .state_trie(base_block_hash)
                    .map_err(|e| RpcErr::Internal(e.to_string()))?
                    .ok_or_else(|| RpcErr::Internal("State trie not found".to_owned()))?;
                state_trie = Some(initial_trie);
                state_trie.as_mut().expect("state_trie was just set above")
            };
            store
                .apply_account_updates_from_trie_batch(trie, &account_updates)
                .map_err(|e| RpcErr::Internal(e.to_string()))?;
            final_header.state_root = trie.hash_no_commit();

            // Now that the block hash is known, compute block size.
            let block_hash = final_header.hash();
            overlay.set_block_hash(final_header.number, block_hash);

            // Compute RLP-encoded block size.
            let block_for_size = Block::new(
                final_header.clone(),
                BlockBody {
                    transactions: transactions.clone(),
                    ommers: vec![],
                    withdrawals: Some(withdrawals.clone()),
                },
            );
            let size = block_for_size.length() as u64;

            // Update log block_hash and tx_hash now that we know them.
            for (tx_idx, call_result) in call_results.iter_mut().enumerate() {
                let tx_hash = tx_infos[tx_idx].hash;
                for log in &mut call_result.logs {
                    log.block_hash = block_hash;
                    log.transaction_hash = tx_hash;
                }
            }

            // Build transactions response field.
            let tx_values: Vec<Value> = if self.payload.return_full_transactions {
                tx_infos
                    .iter()
                    .enumerate()
                    .map(|(tx_idx, ti)| {
                        let (to_addr, access_list, max_fee, max_priority_fee, chain_id_val) =
                            extract_tx_eip1559_fields(&ti.tx, &block_state_call.calls[tx_idx]);
                        let sim_tx = SimulatedTransaction {
                            block_hash,
                            block_number: final_header.number,
                            block_timestamp: final_header.timestamp,
                            from: ti.from,
                            gas: ti.gas_limit,
                            gas_price: ti.gas_price,
                            max_fee_per_gas: max_fee,
                            max_priority_fee_per_gas: max_priority_fee,
                            hash: ti.hash,
                            input: block_state_call.calls[tx_idx].input.clone(),
                            nonce: ti.nonce,
                            to: to_addr,
                            transaction_index: tx_idx as u64,
                            value: block_state_call.calls[tx_idx].value,
                            tx_type: block_state_call.calls[tx_idx].r#type as u64,
                            access_list,
                            chain_id: chain_id_val,
                            v: 0,
                            r: U256::zero(),
                            s: U256::zero(),
                            y_parity: 0,
                        };
                        serde_json::to_value(&sim_tx).unwrap_or(Value::Null)
                    })
                    .collect()
            } else {
                tx_infos
                    .iter()
                    .map(|ti| Value::String(format!("{:#x}", ti.hash)))
                    .collect()
            };

            // 4i. Build response block.
            results.push(SimulatedBlock {
                hash: block_hash,
                size,
                header: final_header.clone(),
                calls: call_results,
                transactions: tx_values,
                uncles: vec![],
                withdrawals,
            });

            // Use final_header (with gas_used set) so parent_hash in the next block
            // matches the hash registered in the overlay for BLOCKHASH lookups.
            prev_header = final_header;
        }

        serde_json::to_value(&results).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}

/// Convert a `GenericTransaction` to an unsigned `Transaction` for hashing and response
/// construction. All signature fields are set to zero/default values.
fn generic_tx_to_unsigned_transaction(
    tx: &GenericTransaction,
    gas_limit: u64,
    chain_id: u64,
) -> Transaction {
    let nonce = tx.nonce.unwrap_or_default();
    let to = tx.to.clone();
    let value = tx.value;
    let data = tx.input.clone();
    let access_list: Vec<(Address, Vec<H256>)> = tx
        .access_list
        .iter()
        .map(|e| (e.address, e.storage_keys.clone()))
        .collect();

    match tx.r#type {
        TxType::Legacy => Transaction::LegacyTransaction(LegacyTransaction {
            nonce,
            gas_price: tx.gas_price.into(),
            gas: gas_limit,
            to,
            value,
            data,
            v: U256::zero(),
            r: U256::zero(),
            s: U256::zero(),
            inner_hash: Default::default(),
            sender_cache: Default::default(),
        }),
        TxType::EIP2930 => Transaction::EIP2930Transaction(EIP2930Transaction {
            chain_id,
            nonce,
            gas_price: tx.gas_price.into(),
            gas_limit,
            to,
            value,
            data,
            access_list,
            signature_y_parity: false,
            signature_r: U256::zero(),
            signature_s: U256::zero(),
            inner_hash: Default::default(),
            sender_cache: Default::default(),
        }),
        TxType::EIP4844 => {
            // EIP-4844 requires a non-null `to` address.
            let to_addr = match to {
                TxKind::Call(addr) => addr,
                TxKind::Create => Address::zero(),
            };
            Transaction::EIP4844Transaction(EIP4844Transaction {
                chain_id,
                nonce,
                max_priority_fee_per_gas: tx.max_priority_fee_per_gas.unwrap_or(0),
                max_fee_per_gas: tx.max_fee_per_gas.unwrap_or(0),
                gas: gas_limit,
                to: to_addr,
                value,
                data,
                access_list,
                max_fee_per_blob_gas: tx.max_fee_per_blob_gas.unwrap_or_default(),
                blob_versioned_hashes: tx.blob_versioned_hashes.clone(),
                signature_y_parity: false,
                signature_r: U256::zero(),
                signature_s: U256::zero(),
                inner_hash: Default::default(),
                sender_cache: Default::default(),
            })
        }
        TxType::EIP7702 => {
            // EIP-7702 requires a non-null `to` address.
            let to_addr = match to {
                TxKind::Call(addr) => addr,
                TxKind::Create => Address::zero(),
            };
            let auth_list: Vec<_> = tx
                .authorization_list
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(|e| e.clone().into())
                .collect();
            Transaction::EIP7702Transaction(EIP7702Transaction {
                chain_id,
                nonce,
                max_priority_fee_per_gas: tx.max_priority_fee_per_gas.unwrap_or(0),
                max_fee_per_gas: tx.max_fee_per_gas.unwrap_or(0),
                gas_limit,
                to: to_addr,
                value,
                data,
                access_list,
                authorization_list: auth_list,
                signature_y_parity: false,
                signature_r: U256::zero(),
                signature_s: U256::zero(),
                inner_hash: Default::default(),
                sender_cache: Default::default(),
            })
        }
        // Default to EIP-1559 for unspecified or EIP-1559 type.
        _ => Transaction::EIP1559Transaction(EIP1559Transaction {
            chain_id,
            nonce,
            max_priority_fee_per_gas: tx.max_priority_fee_per_gas.unwrap_or(0),
            max_fee_per_gas: tx.max_fee_per_gas.unwrap_or(0),
            gas_limit,
            to,
            value,
            data,
            access_list,
            signature_y_parity: false,
            signature_r: U256::zero(),
            signature_s: U256::zero(),
            inner_hash: Default::default(),
            sender_cache: Default::default(),
        }),
    }
}

/// Compute effective gas price for a transaction.
/// For EIP-1559: min(max_fee_per_gas, base_fee + max_priority_fee_per_gas)
/// For Legacy/EIP-2930: gas_price
fn compute_effective_gas_price(tx: &GenericTransaction, base_fee: u64) -> u64 {
    match tx.r#type {
        TxType::EIP1559 | TxType::EIP4844 | TxType::EIP7702 => {
            let max_fee = tx.max_fee_per_gas.unwrap_or(0);
            let max_priority = tx.max_priority_fee_per_gas.unwrap_or(0);
            max_fee.min(base_fee.saturating_add(max_priority))
        }
        _ => tx.gas_price,
    }
}

/// Extract EIP-1559 specific fields from a Transaction for the SimulatedTransaction response.
/// Returns (to, access_list, max_fee_per_gas, max_priority_fee_per_gas, chain_id).
#[allow(clippy::type_complexity)]
fn extract_tx_eip1559_fields(
    tx: &Transaction,
    generic: &GenericTransaction,
) -> (
    Option<Address>,
    Vec<AccessListEntry>,
    Option<U256>,
    Option<U256>,
    Option<U256>,
) {
    let to = match generic.to {
        TxKind::Call(addr) => Some(addr),
        TxKind::Create => None,
    };
    let access_list: Vec<AccessListEntry> = generic
        .access_list
        .iter()
        .map(|e| AccessListEntry {
            address: e.address,
            storage_keys: e.storage_keys.clone(),
        })
        .collect();

    match generic.r#type {
        TxType::EIP1559 | TxType::EIP4844 | TxType::EIP7702 => {
            let max_fee = generic.max_fee_per_gas.map(U256::from);
            let max_priority = generic.max_priority_fee_per_gas.map(U256::from);
            let chain_id = Some(U256::from(match tx {
                Transaction::EIP1559Transaction(t) => t.chain_id,
                Transaction::EIP4844Transaction(t) => t.chain_id,
                Transaction::EIP7702Transaction(t) => t.chain_id,
                Transaction::EIP2930Transaction(t) => t.chain_id,
                _ => 0u64,
            }));
            (to, access_list, max_fee, max_priority, chain_id)
        }
        TxType::EIP2930 => {
            let chain_id = Some(U256::from(match tx {
                Transaction::EIP2930Transaction(t) => t.chain_id,
                _ => 0u64,
            }));
            (to, access_list, None, None, chain_id)
        }
        _ => (to, access_list, None, None, None),
    }
}

/// Create a synthetic traceTransfers log for a top-level ETH value transfer.
#[allow(clippy::too_many_arguments)]
fn create_trace_transfer_log(
    from: Address,
    to: Address,
    value: U256,
    log_index: u64,
    block_number: u64,
    block_timestamp: u64,
    block_hash: H256,
    transaction_hash: H256,
    transaction_index: u64,
) -> SimulatedLog {
    let mut from_topic = [0u8; 32];
    from_topic[12..].copy_from_slice(from.as_bytes());
    let mut to_topic = [0u8; 32];
    to_topic[12..].copy_from_slice(to.as_bytes());

    let value_buf = value.to_big_endian();
    let data = Bytes::from(value_buf.to_vec());

    SimulatedLog {
        address: trace_transfer_address(),
        topics: vec![
            TRANSFER_EVENT_SIGNATURE,
            H256::from(from_topic),
            H256::from(to_topic),
        ],
        data,
        log_index,
        block_number,
        block_hash,
        transaction_hash,
        transaction_index,
        block_timestamp,
        removed: false,
    }
}

/// Build an empty intermediate block (no calls, no state changes).
/// Used for gap filling when blockOverrides.number jumps ahead.
fn build_empty_intermediate_block(
    prev_header: &BlockHeader,
    block_number: u64,
    block_timestamp: u64,
) -> (SimulatedBlock, BlockHeader) {
    let mut header = prev_header.clone();
    header.hash = Default::default();
    header.parent_hash = prev_header.hash();
    header.number = block_number;
    header.timestamp = block_timestamp;
    header.gas_used = 0;
    header.transactions_root = compute_transactions_root(&[]);
    header.receipts_root = compute_receipts_root(&[]);
    header.logs_bloom = Default::default();

    // Set Cancun+ fields to zero values if they're None.
    set_post_cancun_defaults(&mut header);

    let block_hash = header.hash();

    // Compute block size.
    let block_for_size = Block::new(
        header.clone(),
        BlockBody {
            transactions: vec![],
            ommers: vec![],
            withdrawals: Some(vec![]),
        },
    );
    let size = block_for_size.length() as u64;

    let block = SimulatedBlock {
        hash: block_hash,
        size,
        header: header.clone(),
        calls: vec![],
        transactions: vec![],
        uncles: vec![],
        withdrawals: vec![],
    };

    (block, header)
}

/// Set post-Cancun header fields to their zero/empty defaults if they are None.
/// Simulated blocks always include these fields (matching Geth behavior).
fn set_post_cancun_defaults(header: &mut BlockHeader) {
    if header.blob_gas_used.is_none() {
        header.blob_gas_used = Some(0);
    }
    if header.excess_blob_gas.is_none() {
        header.excess_blob_gas = Some(0);
    }
    if header.parent_beacon_block_root.is_none() {
        header.parent_beacon_block_root = Some(H256::zero());
    }
    if header.withdrawals_root.is_none() {
        header.withdrawals_root = Some(*EMPTY_WITHDRAWALS_HASH);
    }
    if header.requests_hash.is_none() {
        header.requests_hash = Some(*DEFAULT_REQUESTS_HASH);
    }
}

/// Build a simulated block header from the previous header and optional overrides.
fn build_simulated_header(
    prev: &BlockHeader,
    overrides: Option<&BlockOverrides>,
) -> Result<BlockHeader, RpcErr> {
    let mut header = prev.clone();
    // Reset cached hash since we're modifying fields.
    header.hash = Default::default();
    header.parent_hash = prev.hash();
    header.number = prev
        .number
        .checked_add(1)
        .ok_or_else(|| RpcErr::BadParams("Block number overflow".to_owned()))?;
    header.timestamp = prev
        .timestamp
        .checked_add(1)
        .ok_or_else(|| RpcErr::BadParams("Timestamp overflow".to_owned()))?;
    header.gas_used = 0;

    if let Some(o) = overrides {
        if let Some(number) = o.number {
            header.number = number;
        }
        if let Some(time) = o.time {
            header.timestamp = time;
        }
        if let Some(gas_limit) = o.gas_limit {
            header.gas_limit = gas_limit;
        }
        if let Some(fee_recipient) = o.fee_recipient {
            header.coinbase = fee_recipient;
        }
        if let Some(prev_randao) = o.prev_randao {
            header.prev_randao = prev_randao;
        }
        if let Some(base_fee) = o.base_fee_per_gas {
            if base_fee > U256::from(u64::MAX) {
                return Err(RpcErr::BadParams("baseFeePerGas overflows u64".to_owned()));
            }
            header.base_fee_per_gas = Some(base_fee.as_u64());
        }
        // blobBaseFee override is applied directly to the EVM environment
        // in the simulation loop, bypassing the excess_blob_gas derivation.
    }

    // Set Cancun+ fields to defaults if missing.
    set_post_cancun_defaults(&mut header);

    Ok(header)
}

/// Validate that block numbers and timestamps are strictly increasing.
fn validate_block_sequence(prev: &BlockHeader, current: &BlockHeader) -> Result<(), RpcErr> {
    if current.number <= prev.number {
        return Err(RpcErr::SimulateError {
            code: -38020,
            message: format!(
                "block numbers must be in order: {} <= {}",
                current.number, prev.number
            ),
        });
    }
    if current.timestamp <= prev.timestamp {
        return Err(RpcErr::SimulateError {
            code: -38021,
            message: format!(
                "block timestamps must be in order: {} <= {}",
                current.timestamp, prev.timestamp
            ),
        });
    }
    Ok(())
}

/// Apply RPC state overrides to the overlay database.
fn apply_state_overrides(
    overlay: &mut OverlayVmDatabase,
    overrides: &HashMap<Address, AccountOverride>,
) {
    for (address, acct_override) in overrides {
        if let Some(balance) = acct_override.balance {
            overlay.set_balance(*address, balance);
        }
        if let Some(nonce) = acct_override.nonce {
            overlay.set_nonce(*address, nonce);
        }
        if let Some(code_bytes) = &acct_override.code {
            overlay.set_code(*address, code_bytes.clone());
        }
        if let Some(full_state) = &acct_override.state {
            let storage: HashMap<H256, U256> = full_state
                .iter()
                .map(|(k, v)| (*k, U256::from_big_endian(v.as_bytes())))
                .collect();
            overlay.set_full_storage(*address, storage);
        }
        if let Some(state_diff) = &acct_override.state_diff {
            let diff: HashMap<H256, U256> = state_diff
                .iter()
                .map(|(k, v)| (*k, U256::from_big_endian(v.as_bytes())))
                .collect();
            overlay.set_storage_diff(*address, diff);
        }
    }
}

/// Convert an `ExecutionResult` (or error) into a `CallResult` for the response.
fn execution_result_to_call_result(
    result: Result<ExecutionResult, impl std::fmt::Display>,
    header: &BlockHeader,
    log_index_offset: u64,
    block_hash: H256,
    transaction_hash: H256,
    transaction_index: u64,
) -> CallResult {
    match result {
        Ok(ExecutionResult::Success {
            gas_used,
            logs,
            output,
            ..
        }) => {
            let sim_logs: Vec<SimulatedLog> = logs
                .iter()
                .enumerate()
                .map(|(i, log)| SimulatedLog {
                    address: log.address,
                    topics: log.topics.clone(),
                    data: log.data.clone(),
                    log_index: log_index_offset + i as u64,
                    block_number: header.number,
                    block_hash,
                    transaction_hash,
                    transaction_index,
                    block_timestamp: header.timestamp,
                    removed: false,
                })
                .collect();

            CallResult {
                status: 1,
                return_data: output,
                gas_used,
                logs: sim_logs,
                error: None,
            }
        }
        Ok(ExecutionResult::Revert { gas_used, output }) => {
            let data = format!("0x{output:x}");
            CallResult {
                status: 0,
                return_data: output,
                gas_used,
                logs: Vec::new(),
                error: Some(CallError {
                    code: 3,
                    message: "execution reverted".to_string(),
                    data: Some(data),
                }),
            }
        }
        Ok(ExecutionResult::Halt { reason, gas_used }) => CallResult {
            status: 0,
            return_data: bytes::Bytes::new(),
            gas_used,
            logs: Vec::new(),
            error: Some(CallError {
                code: -32015,
                message: format!("execution halted: {reason}"),
                data: None,
            }),
        },
        Err(err) => CallResult {
            status: 0,
            return_data: bytes::Bytes::new(),
            gas_used: 0,
            logs: Vec::new(),
            error: Some(CallError {
                code: -32015,
                message: format!("VM error: {err}"),
                data: None,
            }),
        },
    }
}

/// Map VM/EVM errors to eth_simulateV1 specific error codes.
fn map_vm_error_to_simulate_error(err: &impl std::fmt::Display) -> RpcErr {
    let msg = err.to_string();

    // Map known error patterns to specific codes.
    let code = if msg.contains("Nonce mismatch") || msg.contains("nonce too low") {
        -38010
    } else if msg.contains("Nonce is max") || msg.contains("nonce has max value") {
        -32603
    } else if msg.contains("base fee")
        || msg.contains("BaseFeePerGas")
        || msg.contains("max fee per gas")
    {
        -38012
    } else if msg.contains("intrinsic gas") || msg.contains("IntrinsicGasTooLow") {
        -38013
    } else if msg.contains("Insufficient account funds") || msg.contains("insufficient funds") {
        -38014
    } else if msg.contains("gas limit") || msg.contains("GasLimitExceeded") {
        -38015
    } else if msg.contains("not an EOA") || msg.contains("SenderNotEOA") {
        -38024
    } else if msg.contains("init code size") || msg.contains("InitCodeSizeExceeded") {
        -38025
    } else {
        -32015 // fallback
    };

    RpcErr::SimulateError { code, message: msg }
}
