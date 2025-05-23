use std::collections::HashSet;

use ethrex_common::{types::Block, Address, U256};
use revm::Evm;
use revm_inspectors::tracing::{
    types::{CallKind, CallTraceNode},
    CallTraceArena,
};
use revm_primitives::{BlockEnv, ExecutionResult as RevmExecutionResult, SpecId, TxEnv};

use crate::{
    backends::revm::run_evm,
    helpers::spec_id,
    tracing::{Call, CallTrace, CallType},
    EvmError,
};

use super::{block_env, db::EvmState, tx_env, REVM};

impl REVM {
    /// Executes the block until a given tx is reached, then generates the call trace for the tx
    pub fn trace_tx_calls(
        block: &Block,
        tx_index: usize,
        state: &mut EvmState,
    ) -> Result<CallTrace, EvmError> {
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

        let mut call_trace = CallTrace::new();

        for (index, (tx, sender)) in block
            .body
            .get_transactions_with_sender()
            .into_iter()
            .enumerate()
        {
            let tx_env = tx_env(tx, sender);
            if index == tx_index {
                // Trace the transaction
                call_trace = run_evm_with_call_tracer(tx_env, block_env, state, spec_id)?;
                break;
            }
            run_evm(tx_env, block_env.clone(), state, spec_id)?;
        }

        Ok(call_trace)
    }

    /// Reruns the given block, saving the changes on the state, doesn't output any results or receipts
    pub fn rerun_block(block: &Block, state: &mut EvmState) -> Result<(), EvmError> {
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

        for (tx, sender) in block.body.get_transactions_with_sender().into_iter() {
            let tx_env = tx_env(tx, sender);
            run_evm(tx_env, block_env.clone(), state, spec_id)?;
        }

        if let Some(withdrawals) = &block.body.withdrawals {
            Self::process_withdrawals(state, withdrawals)?;
        }

        Ok(())
    }
}

fn run_evm_with_call_tracer(
    tx_env: TxEnv,
    block_env: BlockEnv,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<CallTrace, EvmError> {
    let (call_trace, result) = {
        let chain_spec = state.chain_config()?;
        #[allow(unused_mut)]
        let mut evm_builder = Evm::builder()
            .with_block_env(block_env)
            .with_tx_env(tx_env)
            .modify_cfg_env(|cfg| cfg.chain_id = chain_spec.chain_id)
            .with_spec_id(spec_id)
            .with_external_context(revm_inspectors::tracing::TracingInspector::default());

        match state {
            EvmState::Store(db) => {
                let mut evm = evm_builder.with_db(db).build();
                let res = evm.transact_commit()?;
                let trace = evm.into_context().external.into_traces();
                (trace, res)
            }
            EvmState::Execution(db) => {
                let mut evm = evm_builder.with_db(db).build();
                let res = evm.transact_commit()?;
                let trace = evm.into_context().external.into_traces();
                (trace, res)
            }
        }
    };
    let revert_reason_or_error = result_to_err_or_revert_string(result);
    Ok(map_call_trace(call_trace, &revert_reason_or_error))
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
            output: _,
        } => String::from("Transaction reverted due to revert opcode"),
        RevmExecutionResult::Halt {
            reason,
            gas_used: _,
        } => format!("{reason:?}"),
    }
}

fn map_call_trace(revm_trace: CallTraceArena, revert_reason_or_error: &String) -> CallTrace {
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
) -> Call {
    let mut subcalls = vec![];
    for child_idx in &revm_call.children {
        if let Some(child) = revm_calls.get(*child_idx) {
            subcalls.push(map_call(
                child.clone(),
                revm_calls,
                used_idxs,
                revert_reason_or_error,
            ));
            used_idxs.insert(*child_idx);
        }
    }
    Call {
        r#type: map_call_type(revm_call.kind()),
        from: Address::from_slice(revm_call.trace.caller.0.as_slice()),
        to: Address::from_slice(revm_call.trace.address.0.as_slice()),
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
        calls: Box::new(vec![]),
    }
}

fn map_call_type(revm_call_type: CallKind) -> CallType {
    match revm_call_type {
        CallKind::Call => CallType::Call,
        CallKind::StaticCall => CallType::StaticCall,
        CallKind::CallCode => CallType::Call, //TODO: check this
        CallKind::DelegateCall => CallType::DelegateCall,
        CallKind::AuthCall => CallType::Call, //TODO: check this
        CallKind::Create => CallType::Create,
        CallKind::Create2 => CallType::Create2,
        CallKind::EOFCreate => CallType::Create, //TODO: check this
    }
}
