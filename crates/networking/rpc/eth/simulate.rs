use std::collections::HashMap;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    types::{
        block_identifier::{BlockIdentifier, BlockIdentifierOrHash},
        simulate::{
            AccountOverride, BlockOverrides, CallError, CallResult, SimulatePayload,
            SimulatedBlock, SimulatedLog,
        },
    },
    utils::RpcErr,
};
use ethrex_blockchain::{overlay_vm_db::OverlayVmDatabase, vm::StoreVmDatabase};
use ethrex_common::{
    Address, H256, U256,
    types::BlockHeader,
};
use ethrex_vm::{ExecutionResult, backends::Evm};
use serde_json::Value;
use tracing::debug;

const MAX_BLOCK_STATE_CALLS: usize = 256;

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

        // traceTransfers is accepted but not yet implemented (synthetic transfer
        // logs won't be emitted).  Rejecting it would break many clients.

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

        // 4. Iterate through block state calls.
        for block_state_call in self.payload.block_state_calls.iter() {
            // 4a. Build simulated block header.
            let sim_header = build_simulated_header(
                &prev_header,
                block_state_call.block_overrides.as_ref(),
            )?;

            // 4b. Validate block sequence.
            validate_block_sequence(&prev_header, &sim_header)?;

            // 4c. Apply state overrides for this block.
            if let Some(state_overrides) = &block_state_call.state_overrides {
                apply_state_overrides(&mut overlay, state_overrides);
            }

            // 4d. Create EVM for this block (clone overlay so it stays clean for extraction).
            let mut evm = Evm::new_for_l1(overlay.clone());

            // 4e. Execute each call.
            let mut call_results = Vec::new();
            let mut block_gas_used: u64 = 0;
            let mut cumulative_log_count: u64 = 0;
            let blob_base_fee_override = block_state_call
                .block_overrides
                .as_ref()
                .and_then(|o| o.blob_base_fee);

            for tx in &block_state_call.calls {
                let exec_result = evm.simulate_tx_from_generic_with_validation(
                    tx,
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

                let call_result = execution_result_to_call_result(
                    exec_result,
                    &sim_header,
                    cumulative_log_count,
                    H256::zero(), // placeholder, will be replaced after block hash is known
                );
                block_gas_used += call_result.gas_used;
                cumulative_log_count += call_result.logs.len() as u64;
                call_results.push(call_result);
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

            // 4h. Compute simulated block hash and register it.
            // Note: non-overridable header fields (state_root, transactions_root, etc.)
            // are inherited from the parent and are intentionally stale in the simulated block.
            let mut final_header = sim_header;
            final_header.gas_used = block_gas_used;
            // Reset cached hash since gas_used changed.
            final_header.hash = Default::default();
            let block_hash = final_header.hash();
            overlay.set_block_hash(final_header.number, block_hash);

            // Update log block_hash now that we know it.
            for call_result in &mut call_results {
                for log in &mut call_result.logs {
                    log.block_hash = block_hash;
                }
            }

            // 4i. Build response block.
            results.push(SimulatedBlock {
                hash: block_hash,
                size: 0,
                header: final_header.clone(),
                calls: call_results,
                transactions: vec![],
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

/// Build a simulated block header from the previous header and optional overrides.
fn build_simulated_header(
    prev: &BlockHeader,
    overrides: Option<&BlockOverrides>,
) -> Result<BlockHeader, RpcErr> {
    let mut header = prev.clone();
    // Reset cached hash since we're modifying fields.
    header.hash = Default::default();
    header.parent_hash = prev.hash();
    header.number = prev.number.checked_add(1).ok_or_else(|| {
        RpcErr::BadParams("Block number overflow".to_owned())
    })?;
    header.timestamp = prev.timestamp.checked_add(1).ok_or_else(|| {
        RpcErr::BadParams("Timestamp overflow".to_owned())
    })?;
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
                return Err(RpcErr::BadParams(
                    "baseFeePerGas overflows u64".to_owned(),
                ));
            }
            header.base_fee_per_gas = Some(base_fee.as_u64());
        }
        // blobBaseFee override is applied directly to the EVM environment
        // in the simulation loop, bypassing the excess_blob_gas derivation.
    }

    Ok(header)
}

/// Validate that block numbers and timestamps are strictly increasing.
fn validate_block_sequence(
    prev: &BlockHeader,
    current: &BlockHeader,
) -> Result<(), RpcErr> {
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

    RpcErr::SimulateError {
        code,
        message: msg,
    }
}
