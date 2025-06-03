use ethrex_common::types::Block;
use ethrex_levm::{
    db::gen_db::GeneralizedDatabase,
    opcodes::Opcode,
    tracing::{LevmCallTracer, TracerCallFrame},
    vm::VM,
};

use crate::{
    backends::levm::LEVM,
    tracing::{Call, CallLog, CallTrace, CallType},
    EvmError,
};

impl LEVM {
    /// Execute all transactions of the block up until a certain transaction specified in `stop_index`.
    /// The goal is to just mutate the state up to that point, without needing to process transaction receipts or requests.
    pub fn rerun_block(
        db: &mut GeneralizedDatabase,
        block: &Block,
        stop_index: Option<usize>,
    ) -> Result<(), EvmError> {
        Self::prepare_block(block, db)?;

        // Executes transactions and stops when the index matches the stop index.
        for (index, (tx, sender)) in block
            .body
            .get_transactions_with_sender()
            .into_iter()
            .enumerate()
        {
            if stop_index.is_some_and(|stop| stop == index) {
                break;
            }

            Self::execute_tx(tx, sender, &block.header, db).map_err(EvmError::from)?;
        }

        // Process withdrawals only if the whole block has been executed.
        if stop_index.is_none() {
            if let Some(withdrawals) = &block.body.withdrawals {
                Self::process_withdrawals(db, withdrawals)?;
            }
        };

        Ok(())
    }

    /// Run transaction with callTracer activated.
    pub fn trace_tx_calls(
        db: &mut GeneralizedDatabase,
        block: &Block,
        tx_index: usize,
        only_top_call: bool,
        with_log: bool,
    ) -> Result<CallTrace, EvmError> {
        //TODO: Just send transaction instead of index (?)
        let tx = block
            .body
            .transactions
            .get(tx_index)
            .ok_or(EvmError::Custom(
                "Missing Transaction for Trace".to_string(),
            ))?;

        let env = Self::setup_env(tx, tx.sender(), &block.header, db)?;
        let mut vm = VM::new(env, db, tx, LevmCallTracer::new(only_top_call, with_log));

        vm.execute()?;

        let callframe = vm
            .tracer
            .callframes
            .pop()
            .ok_or(EvmError::Custom("Could not get trace".to_string()))?;

        let call = map_levm_callframe(callframe)?;

        // We only return the top call because a transaction only has one call with subcalls...
        Ok(vec![call])
    }
}

fn map_call_type(opcode: Opcode) -> Result<CallType, EvmError> {
    let call_type = match opcode {
        Opcode::CALL => CallType::Call,
        Opcode::STATICCALL => CallType::StaticCall,
        Opcode::CALLCODE => CallType::CallCode,
        Opcode::DELEGATECALL => CallType::DelegateCall,
        Opcode::CREATE => CallType::Create,
        Opcode::CREATE2 => CallType::Create2,
        Opcode::SELFDESTRUCT => CallType::SelfDestruct,
        _ => return Err(EvmError::Custom("Invalid call type".to_string())),
    };
    Ok(call_type)
}

//TODO: See if we should use the same struct
fn map_call_logs(logs: Vec<ethrex_levm::tracing::TracerLog>) -> Vec<CallLog> {
    logs.into_iter()
        .map(|levm_log| CallLog {
            address: levm_log.address,
            topics: levm_log.topics,
            data: levm_log.data,
            position: levm_log.position as u64, //TODO: u64 or usize?
        })
        .collect()
}

fn map_levm_callframe(callframe: TracerCallFrame) -> Result<Call, EvmError> {
    let mut subcalls = vec![];
    for subcall in callframe.calls {
        subcalls.push(map_levm_callframe(subcall)?);
    }

    let call = Call {
        r#type: map_call_type(callframe.call_type)?,
        from: callframe.from,
        to: callframe.to,
        value: callframe.value,
        gas: callframe.gas,
        gas_used: callframe.gas_used,
        input: callframe.input,
        output: callframe.output,
        error: callframe.error,
        revert_reason: callframe.revert_reason,
        calls: subcalls,
        logs: map_call_logs(callframe.logs),
    };

    Ok(call)
}
