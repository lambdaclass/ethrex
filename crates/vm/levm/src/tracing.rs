use crate::{
    errors::{ExecutionReport, InternalError, TxResult, VMError},
    vm::VM,
};
use bytes::Bytes;
use ethrex_common::{
    tracing::{CallLog, CallTraceFrame, CallType},
    types::Log,
    Address, U256,
};

/// Geth's callTracer (https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers)
/// Use `LevmCallTracer::disabled()` when tracing is not wanted.
#[derive(Debug, Default)]
pub struct LevmCallTracer {
    /// Stack for tracer callframes, at the end of execution there will be only one element.
    pub callframes: Vec<CallTraceFrame>,
    /// If true, trace only the top call (a.k.a. the external transaction)
    pub only_top_call: bool,
    /// If true, trace logs
    pub with_log: bool,
    /// If active is set to false it won't trace.
    pub active: bool,
}

impl LevmCallTracer {
    pub fn new(only_top_call: bool, with_log: bool) -> Self {
        LevmCallTracer {
            callframes: vec![],
            only_top_call,
            with_log,
            active: true,
        }
    }

    /// This is to keep LEVM's code clean, like `self.tracer.enter(...)`,
    /// instead of something more complex or uglier when we don't want to trace.
    /// (For now that we only implement one tracer it may be the most convenient solution)
    pub fn disabled() -> Self {
        LevmCallTracer {
            active: false,
            ..Default::default()
        }
    }

    /// Starts trace call.
    pub fn enter(
        &mut self,
        call_type: CallType,
        from: Address,
        to: Address,
        value: U256,
        gas: u64,
        input: &Bytes, // For avoiding cloning when calling (cleaner code)
    ) {
        if !self.active {
            return;
        }
        if self.only_top_call && !self.callframes.is_empty() {
            // Only create callframe if it's the first one to be created.
            return;
        }

        let callframe = CallTraceFrame {
            call_type,
            from,
            to,
            value,
            gas,
            input: input.clone(),
            ..Default::default()
        };

        self.callframes.push(callframe);
    }

    /// Exits trace call.
    /// Has no validations because it's a private method.
    fn exit(
        &mut self,
        gas_used: u64,
        output: Bytes,
        error: Option<String>,
        revert_reason: Option<String>,
    ) -> Result<(), InternalError> {
        let mut callframe = self
            .callframes
            .pop()
            .ok_or(InternalError::CouldNotPopCallframe)?;

        process_output(&mut callframe, gas_used, output, error, revert_reason);

        // Append executed callframe to parent callframe if appropriate.
        if let Some(parent_callframe) = self.callframes.last_mut() {
            parent_callframe.calls.push(callframe);
        } else {
            self.callframes.push(callframe);
        };
        Ok(())
    }

    /// Exits trace call using the ExecutionReport.
    pub fn exit_report(
        &mut self,
        report: &ExecutionReport,
        is_top_call: bool,
    ) -> Result<(), InternalError> {
        if !self.active {
            return Ok(());
        }
        if self.only_top_call && !is_top_call {
            // We just want to register top call
            return Ok(());
        }
        if is_top_call {
            // After finishing transaction execution clear all logs of callframes that reverted.
            clear_reverted_logs(self.current_callframe_mut()?);
        }
        let (gas_used, output) = (report.gas_used, report.output.clone());

        let (error, revert_reason) = match report.result {
            TxResult::Revert(ref err) => {
                let reason = String::from_utf8(report.output.to_vec()).ok();
                (Some(err.to_string()), reason)
            }
            _ => (None, None),
        };

        self.exit(gas_used, output, error, revert_reason)
    }

    /// Exits trace call when CALL or CREATE opcodes return early or in case SELFDESTRUCT is called.
    pub fn exit_early(
        &mut self,
        gas_used: u64,
        error: Option<String>,
    ) -> Result<(), InternalError> {
        if !self.active || self.only_top_call {
            return Ok(());
        }
        self.exit(gas_used, Bytes::new(), error, None)
    }

    /// Registers log when opcode log is executed.
    /// Note: Logs of callframes that reverted will be removed at end of execution.
    pub fn log(&mut self, log: &Log) -> Result<(), InternalError> {
        if !self.active || !self.with_log {
            return Ok(());
        }
        if self.only_top_call && self.callframes.len() > 1 {
            // Register logs for top call only.
            return Ok(());
        }
        let callframe = self.current_callframe_mut()?;

        let log = CallLog {
            address: log.address,
            topics: log.topics.clone(),
            data: log.data.clone(),
            position: match callframe.calls.len().try_into() {
                Ok(pos) => pos,
                Err(_) => return Err(InternalError::ConversionError),
            },
        };

        callframe.logs.push(log);
        Ok(())
    }

    fn current_callframe_mut(&mut self) -> Result<&mut CallTraceFrame, InternalError> {
        self.callframes
            .last_mut()
            .ok_or(InternalError::CouldNotAccessLastCallframe)
    }
}

fn process_output(
    callframe: &mut CallTraceFrame,
    gas_used: u64,
    output: Bytes,
    error: Option<String>,
    revert_reason: Option<String>,
) {
    callframe.gas_used = gas_used;
    callframe.output = output;
    callframe.error = error;
    callframe.revert_reason = revert_reason;
}

/// Clear logs of callframe if it reverted and repeat the same with its subcalls.
fn clear_reverted_logs(callframe: &mut CallTraceFrame) {
    if callframe.error.is_some() {
        callframe.logs.clear();
    }
    for subcall in &mut callframe.calls {
        clear_reverted_logs(subcall);
    }
}

impl VM<'_> {
    /// This method is intended to be accessed after transaction execution
    pub fn get_trace_result(&mut self) -> Result<CallTraceFrame, VMError> {
        self.tracer
            .callframes
            .pop()
            .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))
    }
}
