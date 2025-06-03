use std::collections::HashSet;

use ethrex_common::{types::Block, Address, H256, U256};
use revm::{inspector_handle_register, Evm};
use revm_inspectors::tracing::{
    types::{CallKind, CallLog as RevmCallLog, CallTraceNode},
    CallTraceArena, TracingInspectorConfig,
};
use revm_primitives::{BlockEnv, ExecutionResult as RevmExecutionResult, SpecId, TxEnv};

use crate::{
    backends::revm::run_evm,
    helpers::spec_id,
    tracing::{Call, CallLog, CallTrace, CallType},
    EvmError,
};

use super::{block_env, db::EvmState, tx_env, REVM};

impl REVM {
    /// Runs a single tx with the call tracer and outputs its trace
    /// Asumes that the received state already contains changes from previous blocks and other
    /// transactions within its block
    pub fn trace_tx_calls(
        block: &Block,
        tx_index: usize,
        state: &mut EvmState,
        only_top_call: bool,
        with_log: bool,
    ) -> Result<CallTrace, EvmError> {
        let spec_id: SpecId = spec_id(&state.chain_config()?, block.header.timestamp);
        let block_env = block_env(&block.header, spec_id);
        let tx = block
            .body
            .transactions
            .get(tx_index)
            .ok_or(EvmError::Custom(
                "Missing Transaction for Trace".to_string(),
            ))?;
        let tx_env = tx_env(tx, tx.sender());
        // Trace the transaction
        run_evm_with_call_tracer(tx_env, block_env, state, spec_id, only_top_call, with_log)
    }

    /// Reruns the given block, saving the changes on the state, doesn't output any results or receipts
    /// If the optional argument `stop_index` is set, the run will stop just before executing the transaction at that index
    /// and won't process the withdrawals afterwards
    pub fn rerun_block(
        block: &Block,
        state: &mut EvmState,
        stop_index: Option<usize>,
    ) -> Result<(), EvmError> {
        let spec_id: SpecId = spec_id(&state.chain_config()?, block.header.timestamp);
        let block_env = block_env(&block.header, spec_id);
        cfg_if::cfg_if! {
            if #[cfg(not(feature = "l2"))] {
                if block.header.parent_beacon_block_root.is_some() && spec_id >= SpecId::CANCUN {
                    Self::beacon_root_contract_call(&block.header, state)?;
                }
                //eip 2935: stores parent block hash in system contract
                if spec_id >= SpecId::PRAGUE {
                    Self::process_block_hash_history(&block.header, state)?;
                }
            }
        }

        for (index, (tx, sender)) in block
            .body
            .get_transactions_with_sender()
            .into_iter()
            .enumerate()
        {
            if stop_index.is_some_and(|stop| stop == index) {
                break;
            }
            let tx_env = tx_env(tx, sender);
            run_evm(tx_env, block_env.clone(), state, spec_id)?;
        }
        if stop_index.is_none() {
            if let Some(withdrawals) = &block.body.withdrawals {
                Self::process_withdrawals(state, withdrawals)?;
            }
        }
        Ok(())
    }
}

fn run_evm_with_call_tracer(
    tx_env: TxEnv,
    block_env: BlockEnv,
    state: &mut EvmState,
    spec_id: SpecId,
    only_top_call: bool,
    with_log: bool,
) -> Result<CallTrace, EvmError> {
    let (call_trace, result) = {
        let chain_spec = state.chain_config()?;
        let config = TracingInspectorConfig {
            record_logs: with_log,
            ..Default::default()
        };
        let evm_builder = Evm::builder()
            .with_block_env(block_env)
            .with_tx_env(tx_env)
            .modify_cfg_env(|cfg| cfg.chain_id = chain_spec.chain_id)
            .with_spec_id(spec_id)
            .with_external_context(revm_inspectors::tracing::TracingInspector::new(config));
        let mut evm = evm_builder
            .with_db(&mut state.inner)
            .append_handler_register(inspector_handle_register)
            .build();
        let res = evm.transact_commit()?;
        let trace = evm.into_context().external.into_traces();
        (trace, res)
    };
    let revert_reason_or_error = result_to_err_or_revert_string(result);
    Ok(map_call_trace(
        call_trace,
        &revert_reason_or_error,
        only_top_call,
        with_log,
    ))
}

fn result_to_err_or_revert_string(result: RevmExecutionResult) -> String {
    match result {
        RevmExecutionResult::Success {
            reason: _,
            gas_used: _,
            gas_refunded: _,
            logs: _,
            output: _,
        } => String::new(),
        RevmExecutionResult::Revert {
            gas_used: _,
            output,
        } => format!(
            "Transaction reverted due to: {}",
            std::str::from_utf8(&output).unwrap_or("unknown")
        ),
        RevmExecutionResult::Halt {
            reason,
            gas_used: _,
        } => format!("{reason:?}"),
    }
}

fn map_call_trace(
    revm_trace: CallTraceArena,
    revert_reason_or_error: &String,
    only_top_call: bool,
    with_log: bool,
) -> CallTrace {
    let mut call_trace = CallTrace::new();
    // Idxs of child calls already included in the parent call
    let mut used_idxs = HashSet::new();
    let revm_calls = revm_trace.into_nodes();
    let revm_calls_copy = revm_calls.clone();
    for revm_call in revm_calls {
        if !used_idxs.contains(&revm_call.idx) {
            call_trace.push(map_call(
                revm_call,
                &revm_calls_copy,
                &mut used_idxs,
                revert_reason_or_error,
                only_top_call,
                with_log,
            ));
        }
    }
    call_trace
}

fn map_call(
    revm_call: CallTraceNode,
    revm_calls: &Vec<CallTraceNode>,
    used_idxs: &mut HashSet<usize>,
    revert_reason_or_error: &String,
    only_top_call: bool,
    with_log: bool,
) -> Call {
    let mut subcalls = vec![];
    if !only_top_call {
        for child_idx in &revm_call.children {
            if let Some(child) = revm_calls.get(*child_idx) {
                subcalls.push(map_call(
                    child.clone(),
                    revm_calls,
                    used_idxs,
                    revert_reason_or_error,
                    only_top_call,
                    with_log,
                ));
                used_idxs.insert(*child_idx);
            }
        }
    }
    let to = Address::from_slice(revm_call.trace.address.0.as_slice());
    Call {
        r#type: map_call_type(revm_call.kind()),
        from: Address::from_slice(revm_call.trace.caller.0.as_slice()),
        to,
        value: U256(*revm_call.trace.value.as_limbs()),
        gas: revm_call.trace.gas_limit,
        gas_used: revm_call.trace.gas_used,
        input: revm_call.trace.data.0.clone(),
        output: revm_call.trace.output.0.clone(),
        error: revm_call
            .status()
            .is_error()
            .then(|| revert_reason_or_error.clone()),
        revert_reason: revm_call
            .status()
            .is_revert()
            .then(|| revert_reason_or_error.clone()),
        calls: subcalls,
        logs: with_log.then(|| {
            revm_call
                .logs
                .into_iter()
                .map(|revm_log| map_log(revm_log, to))
                .collect()
        }),
    }
}

fn map_call_type(revm_call_type: CallKind) -> CallType {
    match revm_call_type {
        CallKind::Call => CallType::Call,
        CallKind::StaticCall => CallType::StaticCall,
        CallKind::CallCode => CallType::CallCode,
        CallKind::DelegateCall => CallType::DelegateCall,
        CallKind::AuthCall => CallType::Call, //TODO: check this
        CallKind::Create => CallType::Create,
        CallKind::Create2 => CallType::Create2,
        CallKind::EOFCreate => CallType::Create, //TODO: check this
    }
}

fn map_log(revm_log: RevmCallLog, address: Address) -> CallLog {
    CallLog {
        address,
        topics: revm_log
            .raw_log
            .topics()
            .iter()
            .map(|t| H256(t.0))
            .collect(),
        data: revm_log.raw_log.data.0,
        position: revm_log.position,
    }
}
