use std::collections::HashMap;

use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    Address, Bloom, Bytes, H160, H256, U256,
    constants::{DEFAULT_OMMERS_HASH, DEFAULT_REQUESTS_HASH, EMPTY_TRIE_HASH},
    serde_utils,
    types::{
        BlockBody, BlockHeader, Code, EIP1559Transaction, ELASTICITY_MULTIPLIER,
        GenericTransaction, Log, Receipt, Transaction, TxKind, TxType, Withdrawal, bloom_from_logs,
        calculate_base_fee_per_gas, compute_receipts_root, compute_transactions_root,
        compute_withdrawals_root,
    },
};
use ethrex_levm::account::AccountStatus;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    types::{
        block::RpcBlock,
        block_identifier::{BlockIdentifier, BlockIdentifierOrHash},
        receipt::RpcLog,
    },
    utils::RpcErr,
};

/// Error codes for eth_simulateV1.
const SIMULATE_BLOCK_NUMBER_ORDER_ERROR: i32 = -38020;
const SIMULATE_TIMESTAMP_ORDER_ERROR: i32 = -38021;

/// Synthetic address used as the emitter for ETH transfer trace logs.
const ETH_TRANSFER_ADDRESS: Address = H160([0xee; 20]);

/// keccak256("Transfer(address,address,uint256)")
const TRANSFER_EVENT_SIGNATURE: H256 = H256([
    0xdd, 0xf2, 0x52, 0xad, 0x1b, 0xe2, 0xc8, 0x9b, 0x69, 0xc2, 0xb0, 0x68, 0xfc, 0x37, 0x8d, 0xaa,
    0x95, 0x2b, 0xa7, 0xf1, 0x63, 0xc4, 0xa1, 0x16, 0x28, 0xf5, 0x5a, 0x4d, 0xf5, 0x23, 0xb3, 0xef,
]);

/// Deserializes an `Option<Bytes>` from a nullable 0x-prefixed hex string.
fn deser_bytes_opt<'de, D>(d: D) -> Result<Option<Bytes>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    let opt = Option::<String>::deserialize(d)?;
    match opt {
        None => Ok(None),
        Some(s) => {
            let raw = hex::decode(s.trim_start_matches("0x"))
                .map_err(|e| D::Error::custom(e.to_string()))?;
            Ok(Some(Bytes::from(raw)))
        }
    }
}

// ── Request types ─────────────────────────────────────────────────────────────

/// Top-level payload for `eth_simulateV1`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EthSimulatePayload {
    pub block_state_calls: Vec<BlockStateCall>,
    #[serde(default)]
    pub trace_transfers: Option<bool>,
    #[serde(default)]
    pub validation: Option<bool>,
    #[serde(default)]
    pub return_full_transactions: Option<bool>,
}

/// One entry in the `blockStateCalls` array.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockStateCall {
    #[serde(default)]
    pub block_overrides: Option<BlockOverrides>,
    #[serde(default)]
    pub state_overrides: Option<HashMap<Address, AccountOverride>>,
    /// Defaults to empty when the field is absent.
    #[serde(default)]
    pub calls: Option<Vec<GenericTransaction>>,
}

/// Block-level fields that can be overridden for a simulated block.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockOverrides {
    #[serde(default, with = "serde_utils::u64::hex_str_opt")]
    pub number: Option<u64>,
    #[serde(default, with = "serde_utils::u64::hex_str_opt")]
    pub time: Option<u64>,
    #[serde(default, with = "serde_utils::u64::hex_str_opt")]
    pub gas_limit: Option<u64>,
    #[serde(default)]
    pub fee_recipient: Option<Address>,
    #[serde(default)]
    pub prev_randao: Option<H256>,
    /// Hex-encoded U256.
    #[serde(default, deserialize_with = "serde_utils::u256::deser_hex_str_opt")]
    pub base_fee_per_gas: Option<U256>,
    #[serde(default, with = "serde_utils::u64::hex_str_opt")]
    pub blob_base_fee: Option<u64>,
    #[serde(default)]
    pub withdrawals: Option<Vec<Withdrawal>>,
}

/// Per-account state that can be overridden before executing a simulated block.
///
/// Either `state` (full replacement) or `state_diff` (patch) may be provided,
/// but not both.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountOverride {
    #[serde(default, with = "serde_utils::u64::hex_str_opt")]
    pub nonce: Option<u64>,
    #[serde(default, deserialize_with = "serde_utils::u256::deser_hex_str_opt")]
    pub balance: Option<U256>,
    #[serde(default, deserialize_with = "deser_bytes_opt")]
    pub code: Option<Bytes>,
    #[serde(default)]
    pub move_precompile_to_address: Option<Address>,
    /// Full storage replacement – all slots not listed are zeroed.
    #[serde(default)]
    pub state: Option<HashMap<H256, H256>>,
    /// Partial storage patch – only the listed slots are modified.
    #[serde(default)]
    pub state_diff: Option<HashMap<H256, H256>>,
}

// ── Response types ────────────────────────────────────────────────────────────

/// Result for one simulated block returned by `eth_simulateV1`.
#[derive(Debug, Serialize)]
pub struct SimulateBlockResult {
    #[serde(flatten)]
    pub block: RpcBlock,
    pub calls: Vec<SimulateCallResult>,
}

/// Result for a single call within a simulated block.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum SimulateCallResult {
    Success(SimulateCallSuccess),
    Failure(SimulateCallFailure),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulateCallSuccess {
    /// Always `"0x1"`.
    pub status: String,
    #[serde(with = "serde_utils::bytes")]
    pub return_data: Bytes,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub gas_used: u64,
    pub logs: Vec<RpcLog>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulateCallFailure {
    /// Always `"0x0"`.
    pub status: String,
    #[serde(with = "serde_utils::bytes")]
    pub return_data: Bytes,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub gas_used: u64,
    pub error: SimulateCallError,
}

#[derive(Debug, Serialize)]
pub struct SimulateCallError {
    pub code: i32,
    pub message: String,
}

// ── RPC handler ───────────────────────────────────────────────────────────────

pub struct SimulateV1Request {
    pub payload: EthSimulatePayload,
    pub block: Option<BlockIdentifierOrHash>,
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
        let payload: EthSimulatePayload = serde_json::from_value(params[0].clone())?;
        let block = match params.get(1) {
            Some(value) => Some(BlockIdentifierOrHash::parse(value.clone(), 1)?),
            None => None,
        };
        Ok(Self { payload, block })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let block = self
            .block
            .clone()
            .unwrap_or(BlockIdentifierOrHash::Identifier(BlockIdentifier::default()));
        let base_header = match block.resolve_block_header(&context.storage).await? {
            Some(header) => header,
            _ => return Ok(Value::Null),
        };

        let validation = self.payload.validation.unwrap_or(false);
        let trace_transfers = self.payload.trace_transfers.unwrap_or(false);
        let return_full_transactions = self.payload.return_full_transactions.unwrap_or(false);

        let vm_db = StoreVmDatabase::new(context.storage.clone(), base_header.clone())?;
        let mut vm = context.blockchain.new_evm(vm_db)?;

        let mut results: Vec<SimulateBlockResult> = Vec::new();
        let mut prev_header = base_header;

        for block_state_call in &self.payload.block_state_calls {
            // Build the synthetic header for this block.
            let header = build_synthetic_header(
                &prev_header,
                &block_state_call.block_overrides,
                validation,
            )?;

            // Apply state overrides.
            // movePrecompileToAddress is processed first (per spec).
            if let Some(overrides) = &block_state_call.state_overrides {
                apply_precompile_moves(&mut vm.db, overrides)?;
                apply_state_overrides(&mut vm.db, overrides)?;
            }

            // Execute each call and collect results.
            let calls = block_state_call.calls.as_deref().unwrap_or(&[]);
            let withdrawals = block_state_call
                .block_overrides
                .as_ref()
                .and_then(|o| o.withdrawals.clone())
                .unwrap_or_default();

            let mut call_results: Vec<SimulateCallResult> = Vec::new();
            let mut block_transactions: Vec<Transaction> = Vec::new();
            let mut block_tx_senders: Vec<Address> = Vec::new();
            let mut all_logs: Vec<Log> = Vec::new();
            let mut cumulative_gas_used: u64 = 0;
            let mut receipts: Vec<Receipt> = Vec::new();

            for (tx_index, tx) in calls.iter().enumerate() {
                let mut tx = tx.clone();
                // Fill in nonce from account state if not provided.
                if tx.nonce.is_none() {
                    let nonce = vm
                        .db
                        .get_account(tx.from)
                        .map(|a| a.info.nonce)
                        .unwrap_or(0);
                    tx.nonce = Some(nonce);
                }
                // Use block gas limit as default gas if not set.
                if tx.gas.is_none() {
                    tx.gas = Some(header.gas_limit);
                }

                let block_hash = header.hash();
                let block_number = header.number;

                let exec = if validation {
                    vm.simulate_tx_from_generic_validated(&tx, &header)
                } else {
                    vm.simulate_tx_from_generic(&tx, &header)
                };
                match exec {
                    Ok(exec_result) => {
                        let gas_used = exec_result.gas_used();
                        cumulative_gas_used = cumulative_gas_used.saturating_add(gas_used);

                        let succeeded = exec_result.is_success();
                        let (output, tx_logs) = match &exec_result {
                            ethrex_vm::ExecutionResult::Success { output, logs, .. } => {
                                (output.clone(), logs.clone())
                            }
                            ethrex_vm::ExecutionResult::Revert { output, .. } => {
                                (output.clone(), vec![])
                            }
                            ethrex_vm::ExecutionResult::Halt { .. } => (Bytes::new(), vec![]),
                        };

                        let synthetic_tx = generic_to_transaction(&tx);
                        let tx_hash = synthetic_tx.hash();

                        // Build RpcLogs with block/tx context.
                        let mut rpc_logs: Vec<RpcLog> = Vec::new();
                        for (log_index_in_tx, log) in tx_logs.iter().enumerate() {
                            rpc_logs.push(build_rpc_log(
                                log.clone(),
                                (all_logs.len() + log_index_in_tx) as u64,
                                tx_hash,
                                tx_index as u64,
                                block_hash,
                                block_number,
                            ));
                        }

                        // ETH transfer trace logs are injected into call results
                        // but NOT into all_logs (they must not affect logsBloom).
                        if trace_transfers && succeeded && !tx.value.is_zero() {
                            if let TxKind::Call(to_addr) = tx.to {
                                let eth_log = create_eth_transfer_log(tx.from, to_addr, tx.value);
                                let log_index = (all_logs.len() + tx_logs.len()) as u64;
                                rpc_logs.push(build_rpc_log(
                                    eth_log,
                                    log_index,
                                    tx_hash,
                                    tx_index as u64,
                                    block_hash,
                                    block_number,
                                ));
                            }
                        }

                        all_logs.extend(tx_logs.clone());
                        receipts.push(Receipt::new(
                            TxType::EIP1559,
                            succeeded,
                            cumulative_gas_used,
                            tx_logs,
                        ));
                        block_transactions.push(synthetic_tx);
                        block_tx_senders.push(tx.from);

                        if succeeded {
                            call_results.push(SimulateCallResult::Success(SimulateCallSuccess {
                                status: "0x1".to_owned(),
                                return_data: output,
                                gas_used,
                                logs: rpc_logs,
                            }));
                        } else {
                            call_results.push(SimulateCallResult::Failure(SimulateCallFailure {
                                status: "0x0".to_owned(),
                                return_data: output,
                                gas_used,
                                error: SimulateCallError {
                                    code: 3,
                                    message: "execution reverted".to_owned(),
                                },
                            }));
                        }
                    }
                    Err(e) => {
                        call_results.push(SimulateCallResult::Failure(SimulateCallFailure {
                            status: "0x0".to_owned(),
                            return_data: Bytes::new(),
                            gas_used: 0,
                            error: SimulateCallError {
                                code: -32000,
                                message: e.to_string(),
                            },
                        }));
                        receipts.push(Receipt::new(
                            TxType::EIP1559,
                            false,
                            cumulative_gas_used,
                            vec![],
                        ));
                        block_transactions.push(generic_to_transaction(&tx));
                        block_tx_senders.push(tx.from);
                    }
                }
            }

            // Compute account updates and state root for this block.
            // Use the readonly variant so the cache is NOT cleared — state
            // must persist across blocks in the simulation.
            let account_updates = vm.get_state_transitions_readonly()?;
            let state_root = context
                .storage
                .apply_account_updates_batch(prev_header.hash(), &account_updates)
                .map(|opt| {
                    opt.map(|u| u.state_trie_hash)
                        .unwrap_or(prev_header.state_root)
                })
                .unwrap_or(prev_header.state_root);

            // Compute block-level fields.
            let transactions_root = compute_transactions_root(&block_transactions);
            let receipts_root = compute_receipts_root(&receipts);
            let withdrawals_root = compute_withdrawals_root(&withdrawals);
            let logs_bloom = bloom_from_logs(&all_logs);

            // Finalize the header with computed fields.
            let final_header = BlockHeader {
                state_root,
                transactions_root,
                receipts_root,
                logs_bloom,
                gas_used: cumulative_gas_used,
                withdrawals_root: Some(withdrawals_root),
                ..header
            };

            let block_body = BlockBody {
                transactions: block_transactions,
                ommers: vec![],
                withdrawals: Some(withdrawals),
            };

            let block_hash = final_header.hash();

            // Register this simulated block's hash so BLOCKHASH opcode
            // in subsequent blocks can resolve it.
            vm.db
                .block_hash_overrides
                .insert(final_header.number, block_hash);

            let rpc_block = build_simulate_rpc_block(
                final_header.clone(),
                block_body,
                block_hash,
                return_full_transactions,
                &block_tx_senders,
            )?;

            results.push(SimulateBlockResult {
                block: rpc_block,
                calls: call_results,
            });

            // The next block's parent is this simulated block.
            // State persists in the GeneralizedDatabase cache across blocks.
            prev_header = final_header;
        }

        serde_json::to_value(results).map_err(|e| RpcErr::Internal(e.to_string()))
    }
}

/// Builds a synthetic block header for a simulated block.
fn build_synthetic_header(
    parent: &BlockHeader,
    overrides: &Option<BlockOverrides>,
    validation: bool,
) -> Result<BlockHeader, RpcErr> {
    let number = overrides
        .as_ref()
        .and_then(|o| o.number)
        .unwrap_or(parent.number + 1);

    // Block numbers must strictly increase.
    if number <= parent.number {
        return Err(RpcErr::Simulate {
            code: SIMULATE_BLOCK_NUMBER_ORDER_ERROR,
            message: format!(
                "block number {} is not greater than parent {}",
                number, parent.number
            ),
        });
    }

    let timestamp = overrides
        .as_ref()
        .and_then(|o| o.time)
        .unwrap_or(parent.timestamp + 12);

    // Timestamps must be non-decreasing.
    if timestamp < parent.timestamp {
        return Err(RpcErr::Simulate {
            code: SIMULATE_TIMESTAMP_ORDER_ERROR,
            message: format!(
                "block timestamp {} is less than parent {}",
                timestamp, parent.timestamp
            ),
        });
    }

    let gas_limit = overrides
        .as_ref()
        .and_then(|o| o.gas_limit)
        .unwrap_or(parent.gas_limit);

    let coinbase = overrides
        .as_ref()
        .and_then(|o| o.fee_recipient)
        .unwrap_or(Address::zero());

    let prev_randao = overrides
        .as_ref()
        .and_then(|o| o.prev_randao)
        .unwrap_or(H256::zero());

    let base_fee_per_gas =
        if let Some(override_fee) = overrides.as_ref().and_then(|o| o.base_fee_per_gas) {
            Some(override_fee.as_u64())
        } else if validation {
            // In validation mode compute from parent.
            calculate_base_fee_per_gas(
                gas_limit,
                parent.gas_limit,
                parent.gas_used,
                parent.base_fee_per_gas.unwrap_or(0),
                ELASTICITY_MULTIPLIER,
            )
        } else {
            // In non-validation mode disable base fee.
            Some(0)
        };

    let excess_blob_gas = if validation {
        // In validation mode, propagate parent's excess blob gas.
        // Full calc_excess_blob_gas needs ForkBlobSchedule which requires chain config;
        // for now propagate the parent value (correct when parent blob_gas_used == target).
        Some(parent.excess_blob_gas.unwrap_or(0))
    } else {
        // In non-validation mode, set to 0 so blob base fee is 0.
        Some(0)
    };

    let withdrawals_root = overrides
        .as_ref()
        .and_then(|o| o.withdrawals.as_ref())
        .map(|w| compute_withdrawals_root(w))
        .unwrap_or(*EMPTY_TRIE_HASH);

    Ok(BlockHeader {
        parent_hash: parent.hash(),
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase,
        // Will be recomputed after execution.
        state_root: parent.state_root,
        // Will be recomputed after execution.
        transactions_root: H256::zero(),
        // Will be recomputed after execution.
        receipts_root: H256::zero(),
        // Will be recomputed after execution.
        logs_bloom: Bloom::zero(),
        difficulty: U256::zero(),
        number,
        gas_limit,
        // Will be recomputed after execution.
        gas_used: 0,
        timestamp,
        extra_data: Bytes::new(),
        prev_randao,
        nonce: 0,
        base_fee_per_gas,
        withdrawals_root: Some(withdrawals_root),
        blob_gas_used: Some(0),
        excess_blob_gas,
        parent_beacon_block_root: Some(H256::zero()),
        requests_hash: Some(*DEFAULT_REQUESTS_HASH),
        block_access_list_hash: None,
        slot_number: None,
        hash: Default::default(),
    })
}

/// Processes `movePrecompileToAddress` directives from state overrides.
/// Must be called BEFORE `apply_state_overrides` so code overrides on the
/// original precompile address take effect after the move.
fn apply_precompile_moves(
    db: &mut GeneralizedDatabase,
    overrides: &HashMap<Address, AccountOverride>,
) -> Result<(), RpcErr> {
    let mut move_targets: HashMap<Address, Address> = HashMap::new();

    for (address, account_override) in overrides {
        if let Some(target) = account_override.move_precompile_to_address {
            // -38022: movePrecompileToAddress referenced itself
            if target == *address {
                return Err(RpcErr::Simulate {
                    code: -38022,
                    message: format!("movePrecompileToAddress for {address} references itself"),
                });
            }
            // -38023: multiple overrides referencing the same target
            if move_targets.values().any(|&existing| existing == target) {
                return Err(RpcErr::Simulate {
                    code: -38023,
                    message: format!("multiple movePrecompileToAddress entries target {target}"),
                });
            }
            move_targets.insert(*address, target);
        }
    }

    // Register the mappings: target_address → original_precompile_address.
    // The original address is the key in the overrides map (e.g., 0x01 for ecrecover).
    // The target is where the precompile should be accessible.
    for (original_precompile, target_addr) in &move_targets {
        db.precompile_overrides
            .insert(*target_addr, *original_precompile);
    }

    Ok(())
}

/// Applies per-account state overrides directly into the VM database cache.
fn apply_state_overrides(
    db: &mut GeneralizedDatabase,
    overrides: &HashMap<Address, AccountOverride>,
) -> Result<(), RpcErr> {
    for (address, account_override) in overrides {
        // Ensure the account is loaded into the cache first.
        let _ = db
            .get_account_mut(*address)
            .map_err(|e| RpcErr::Internal(e.to_string()))?;

        let account = db.current_accounts_state.get_mut(address).ok_or_else(|| {
            RpcErr::Internal(format!("account {address} not in cache after load"))
        })?;

        if let Some(balance) = account_override.balance {
            account.info.balance = balance;
        }

        if let Some(nonce) = account_override.nonce {
            account.info.nonce = nonce;
        }

        if let Some(code_bytes) = &account_override.code {
            let code = Code::from_bytecode(code_bytes.clone());
            let code_hash = code.hash;
            db.codes.insert(code_hash, code);
            // Re-borrow after inserting into codes.
            let account = db
                .current_accounts_state
                .get_mut(address)
                .ok_or_else(|| RpcErr::Internal(format!("account {address} not in cache")))?;
            account.info.code_hash = code_hash;
        }

        // For full storage replacement: clear all slots and set the new ones.
        if let Some(new_state) = &account_override.state {
            let account = db
                .current_accounts_state
                .get_mut(address)
                .ok_or_else(|| RpcErr::Internal(format!("account {address} not in cache")))?;
            account.storage.clear();
            account.has_storage = !new_state.is_empty();
            for (slot, value) in new_state {
                account
                    .storage
                    .insert(*slot, U256::from_big_endian(value.as_bytes()));
            }
        }

        // For partial storage patch: overwrite only specified slots.
        if let Some(diff) = &account_override.state_diff {
            let account = db
                .current_accounts_state
                .get_mut(address)
                .ok_or_else(|| RpcErr::Internal(format!("account {address} not in cache")))?;
            for (slot, value) in diff {
                account
                    .storage
                    .insert(*slot, U256::from_big_endian(value.as_bytes()));
            }
            if !diff.is_empty() {
                account.has_storage = true;
            }
        }

        // Mark account as modified so state transitions pick it up.
        let account = db
            .current_accounts_state
            .get_mut(address)
            .ok_or_else(|| RpcErr::Internal(format!("account {address} not in cache")))?;
        account.status = AccountStatus::Modified;
    }

    Ok(())
}

/// Converts a `GenericTransaction` to a canonical `Transaction` (EIP-1559)
/// for the purpose of computing transaction hashes and building block bodies.
fn generic_to_transaction(tx: &GenericTransaction) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: tx.chain_id.unwrap_or(1),
        nonce: tx.nonce.unwrap_or(0),
        max_priority_fee_per_gas: tx.max_priority_fee_per_gas.unwrap_or(0),
        max_fee_per_gas: tx.max_fee_per_gas.unwrap_or(0),
        gas_limit: tx.gas.unwrap_or(0),
        to: tx.to.clone(),
        value: tx.value,
        data: tx.input.clone(),
        access_list: tx
            .access_list
            .iter()
            .map(|e| (e.address, e.storage_keys.clone()))
            .collect(),
        ..Default::default()
    })
}

/// Builds an `RpcLog` with block/tx context for the simulate response.
fn build_rpc_log(
    log: Log,
    log_index: u64,
    transaction_hash: H256,
    transaction_index: u64,
    block_hash: H256,
    block_number: u64,
) -> RpcLog {
    use crate::types::receipt::RpcLogInfo;

    RpcLog {
        log: RpcLogInfo::from(log),
        log_index,
        removed: false,
        transaction_hash,
        transaction_index,
        block_hash,
        block_number,
    }
}

/// Builds an RpcBlock for simulation results.
/// Unlike `RpcBlock::build`, this does NOT call `tx.sender()` for full tx objects,
/// since simulated transactions have zero signatures (no ECDSA recovery possible).
fn build_simulate_rpc_block(
    header: BlockHeader,
    body: BlockBody,
    hash: H256,
    full_transactions: bool,
    senders: &[Address],
) -> Result<RpcBlock, RpcErr> {
    use crate::types::block::{BlockBodyWrapper, FullBlockBody, OnlyHashesBlockBody};
    use crate::types::transaction::RpcTransaction;
    use ethrex_rlp::encode::RLPEncode as _;

    let size = ethrex_common::types::Block::new(header.clone(), body.clone()).length() as u64;

    let body_wrapper = if full_transactions {
        let mut transactions = Vec::new();
        for (index, tx) in body.transactions.iter().enumerate() {
            let from = senders.get(index).copied().unwrap_or_default();
            transactions.push(RpcTransaction {
                hash: tx.hash(),
                tx: tx.clone(),
                block_number: Some(header.number),
                block_hash: Some(hash),
                from,
                transaction_index: Some(index as u64),
            });
        }
        BlockBodyWrapper::Full(FullBlockBody {
            transactions,
            uncles: body.ommers.iter().map(|o| o.hash()).collect(),
            withdrawals: body.withdrawals.unwrap_or_default(),
        })
    } else {
        BlockBodyWrapper::OnlyHashes(OnlyHashesBlockBody {
            transactions: body.transactions.iter().map(|t| t.hash()).collect(),
            uncles: body.ommers.iter().map(|o| o.hash()).collect(),
            withdrawals: body.withdrawals.unwrap_or_default(),
        })
    };

    Ok(RpcBlock {
        hash,
        size,
        header,
        body: body_wrapper,
    })
}

/// Creates a synthetic ETH transfer log (ERC20 Transfer event from 0xeeee...eeee).
fn create_eth_transfer_log(from: Address, to: Address, value: U256) -> Log {
    // topic[1] = sender address, left-padded to 32 bytes
    let mut from_topic = [0u8; 32];
    from_topic[12..32].copy_from_slice(from.as_bytes());

    // topic[2] = receiver address, left-padded to 32 bytes
    let mut to_topic = [0u8; 32];
    to_topic[12..32].copy_from_slice(to.as_bytes());

    // data = value as 32-byte big-endian
    let data: [u8; 32] = value.to_big_endian();

    Log {
        address: ETH_TRANSFER_ADDRESS,
        topics: vec![
            TRANSFER_EVENT_SIGNATURE,
            H256::from(from_topic),
            H256::from(to_topic),
        ],
        data: Bytes::from(data.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Payload parsing ────────────────────────────────────────────────────────

    #[test]
    fn test_parse_empty_payload() {
        let json = r#"{"blockStateCalls":[{}]}"#;
        let payload: EthSimulatePayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.block_state_calls.len(), 1);
        assert!(payload.validation.is_none());
        assert!(payload.trace_transfers.is_none());
    }

    #[test]
    fn test_parse_full_payload() {
        let json = r#"{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": {
                        "balance": "0x3e8"
                    }
                },
                "calls": [{
                    "from": "0xc000000000000000000000000000000000000000",
                    "to": "0xc100000000000000000000000000000000000000",
                    "value": "0x3e8"
                }]
            }],
            "traceTransfers": true,
            "validation": false,
            "returnFullTransactions": true
        }"#;
        let payload: EthSimulatePayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.block_state_calls.len(), 1);
        assert_eq!(payload.trace_transfers, Some(true));
        assert_eq!(payload.validation, Some(false));
        assert_eq!(payload.return_full_transactions, Some(true));

        let bsc = &payload.block_state_calls[0];
        assert!(bsc.state_overrides.is_some());
        let overrides = bsc.state_overrides.as_ref().unwrap();
        assert_eq!(overrides.len(), 1);

        let calls = bsc.calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].value, U256::from(0x3e8u64));
    }

    #[test]
    fn test_parse_block_overrides() {
        let json = r#"{
            "blockStateCalls": [{
                "blockOverrides": {
                    "number": "0x14",
                    "time": "0xc8",
                    "gasLimit": "0x2e631",
                    "feeRecipient": "0xc100000000000000000000000000000000000000",
                    "prevRandao": "0x0000000000000000000000000000000000000000000000000000000000001234",
                    "baseFeePerGas": "0x14",
                    "blobBaseFee": "0x15"
                }
            }]
        }"#;
        let payload: EthSimulatePayload = serde_json::from_str(json).unwrap();
        let overrides = payload.block_state_calls[0]
            .block_overrides
            .as_ref()
            .unwrap();
        assert_eq!(overrides.number, Some(0x14));
        assert_eq!(overrides.time, Some(0xc8));
        assert_eq!(overrides.gas_limit, Some(0x2e631));
        assert_eq!(overrides.base_fee_per_gas, Some(U256::from(0x14u64)));
        assert_eq!(overrides.blob_base_fee, Some(0x15));
    }

    #[test]
    fn test_parse_account_override_state() {
        let json = r#"{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": {
                        "balance": "0x7d0",
                        "nonce": "0xa",
                        "code": "0x600160005260206000f3",
                        "state": {
                            "0x0000000000000000000000000000000000000000000000000000000000000000": "0x0000000000000000000000000000000000000000000000000000000000000001"
                        }
                    }
                }
            }]
        }"#;
        let payload: EthSimulatePayload = serde_json::from_str(json).unwrap();
        let overrides = payload.block_state_calls[0]
            .state_overrides
            .as_ref()
            .unwrap();
        let addr: Address = "0xc000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let account = overrides.get(&addr).unwrap();
        assert_eq!(account.balance, Some(U256::from(0x7d0u64)));
        assert_eq!(account.nonce, Some(0xa));
        assert!(account.code.is_some());
        assert!(account.state.is_some());
        assert_eq!(account.state.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_parse_account_override_state_diff() {
        let json = r#"{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0xc000000000000000000000000000000000000000": {
                        "stateDiff": {
                            "0x0000000000000000000000000000000000000000000000000000000000000001": "0x0000000000000000000000000000000000000000000000000000000000000042"
                        }
                    }
                }
            }]
        }"#;
        let payload: EthSimulatePayload = serde_json::from_str(json).unwrap();
        let overrides = payload.block_state_calls[0]
            .state_overrides
            .as_ref()
            .unwrap();
        let addr: Address = "0xc000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let account = overrides.get(&addr).unwrap();
        assert!(account.state_diff.is_some());
        assert_eq!(account.state_diff.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_parse_move_precompile() {
        let json = r#"{
            "blockStateCalls": [{
                "stateOverrides": {
                    "0x0000000000000000000000000000000000000001": {
                        "movePrecompileToAddress": "0x0000000000000000000000000000000000123456"
                    }
                }
            }]
        }"#;
        let payload: EthSimulatePayload = serde_json::from_str(json).unwrap();
        let overrides = payload.block_state_calls[0]
            .state_overrides
            .as_ref()
            .unwrap();
        let addr: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let account = overrides.get(&addr).unwrap();
        assert!(account.move_precompile_to_address.is_some());
        let target: Address = "0x0000000000000000000000000000000000123456"
            .parse()
            .unwrap();
        assert_eq!(account.move_precompile_to_address, Some(target));
    }

    // ── Response serialization ─────────────────────────────────────────────────

    #[test]
    fn test_serialize_call_success() {
        let result = SimulateCallSuccess {
            status: "0x1".to_owned(),
            return_data: Bytes::from_static(b""),
            gas_used: 0x5208,
            logs: vec![],
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["status"], "0x1");
        assert_eq!(json["returnData"], "0x");
        assert_eq!(json["gasUsed"], "0x5208");
    }

    #[test]
    fn test_serialize_call_failure() {
        let result = SimulateCallFailure {
            status: "0x0".to_owned(),
            return_data: Bytes::from_static(b""),
            gas_used: 0,
            error: SimulateCallError {
                code: 3,
                message: "execution reverted".to_owned(),
            },
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["status"], "0x0");
        assert_eq!(json["error"]["code"], 3);
    }

    // ── ETH transfer log ───────────────────────────────────────────────────────

    #[test]
    fn test_create_eth_transfer_log() {
        let from: Address = "0xc000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let to: Address = "0xc100000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let value = U256::from(0x3e8u64);

        let log = create_eth_transfer_log(from, to, value);

        assert_eq!(log.address, ETH_TRANSFER_ADDRESS);

        assert_eq!(log.topics.len(), 3);
        assert_eq!(log.topics[0], TRANSFER_EVENT_SIGNATURE);

        let mut expected_from = [0u8; 32];
        expected_from[12..32].copy_from_slice(from.as_bytes());
        assert_eq!(log.topics[1], H256::from(expected_from));

        let mut expected_to = [0u8; 32];
        expected_to[12..32].copy_from_slice(to.as_bytes());
        assert_eq!(log.topics[2], H256::from(expected_to));

        assert_eq!(log.data.len(), 32);
    }

    // ── build_synthetic_header ─────────────────────────────────────────────────

    fn default_block_overrides() -> BlockOverrides {
        BlockOverrides {
            number: None,
            time: None,
            gas_limit: None,
            fee_recipient: None,
            prev_randao: None,
            base_fee_per_gas: None,
            blob_base_fee: None,
            withdrawals: None,
        }
    }

    #[test]
    fn test_build_synthetic_header_defaults() {
        let parent = BlockHeader {
            number: 100,
            timestamp: 1000,
            gas_limit: 30_000_000,
            gas_used: 15_000_000,
            base_fee_per_gas: Some(1000),
            ..Default::default()
        };

        let header = build_synthetic_header(&parent, &None, false).unwrap();
        assert_eq!(header.number, 101);
        assert_eq!(header.timestamp, 1012);
        assert_eq!(header.gas_limit, 30_000_000);
        // In non-validation mode, base fee is forced to 0.
        assert_eq!(header.base_fee_per_gas, Some(0));
        assert_eq!(header.parent_hash, parent.hash());
    }

    #[test]
    fn test_build_synthetic_header_block_number_order_error() {
        let parent = BlockHeader {
            number: 100,
            ..Default::default()
        };
        let overrides = Some(BlockOverrides {
            number: Some(99), // less than parent
            ..default_block_overrides()
        });
        let result = build_synthetic_header(&parent, &overrides, false);
        assert!(result.is_err());
        match result.unwrap_err() {
            RpcErr::Simulate { code, .. } => {
                assert_eq!(code, SIMULATE_BLOCK_NUMBER_ORDER_ERROR);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn test_build_synthetic_header_timestamp_order_error() {
        let parent = BlockHeader {
            number: 100,
            timestamp: 1000,
            ..Default::default()
        };
        let overrides = Some(BlockOverrides {
            number: Some(101),
            time: Some(999), // less than parent timestamp
            ..default_block_overrides()
        });
        let result = build_synthetic_header(&parent, &overrides, false);
        assert!(result.is_err());
        match result.unwrap_err() {
            RpcErr::Simulate { code, .. } => {
                assert_eq!(code, SIMULATE_TIMESTAMP_ORDER_ERROR);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    // ── SimulateV1Request parsing ──────────────────────────────────────────────

    #[test]
    fn test_simulate_request_parse() {
        let params = vec![
            serde_json::json!({
                "blockStateCalls": [{}]
            }),
            serde_json::json!("latest"),
        ];
        let req = SimulateV1Request::parse(&Some(params)).unwrap();
        assert_eq!(req.payload.block_state_calls.len(), 1);
        assert!(req.block.is_some());
    }

    #[test]
    fn test_simulate_request_parse_no_params() {
        let result = SimulateV1Request::parse(&None);
        assert!(result.is_err());
    }

    // Integration tests are in test/tests/rpc/simulate_tests.rs
}
