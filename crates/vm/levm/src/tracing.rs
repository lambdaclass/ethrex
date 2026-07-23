pub use crate::opcode_tracer::{LevmOpcodeTracer, OpcodeTracerConfig};
use crate::{
    errors::{ContextResult, InternalError, TxResult, VMError},
    vm::VM,
};
use bytes::Bytes;
use ethrex_common::{
    Address, U256,
    tracing::{CallLog, CallTraceFrame, CallType},
    types::Log,
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
    /// Next block-absolute log index to assign (geth's `log.Index`). Seeded with the
    /// count of logs emitted by preceding txs in the block and bumped on each log.
    pub next_log_index: u64,
}

impl LevmCallTracer {
    /// `log_index_base` is the number of logs emitted by preceding txs in the block, so
    /// the first log this tx traces gets geth's block-absolute index. Pass 0 when there
    /// is no preceding context (e.g. the first tx, or `with_log` disabled).
    pub fn new(only_top_call: bool, with_log: bool, log_index_base: u64) -> Self {
        LevmCallTracer {
            callframes: vec![],
            only_top_call,
            with_log,
            active: true,
            next_log_index: log_index_base,
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

        // geth traces STATICCALL with a nil value; every other call type carries one.
        let value = if matches!(call_type, CallType::STATICCALL) {
            None
        } else {
            Some(value)
        };

        let callframe = CallTraceFrame {
            call_type,
            from,
            to: Some(to),
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
        let mut callframe = self.callframes.pop().ok_or(InternalError::CallFrame)?;

        process_output(&mut callframe, gas_used, output, error, revert_reason);

        // Append executed callframe to parent callframe if appropriate.
        if let Some(parent_callframe) = self.callframes.last_mut() {
            parent_callframe.calls.push(callframe);
        } else {
            self.callframes.push(callframe);
        };
        Ok(())
    }

    /// Exits trace call using the ContextResult.
    pub fn exit_context(
        &mut self,
        ctx_result: &ContextResult,
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
            // After finishing transaction execution clear all logs of callframes that reverted,
            // then assign block-absolute indices to the survivors in emission order.
            clear_reverted_logs(self.current_callframe_mut()?, false);
            let mut next_index = self.next_log_index;
            assign_log_indices(self.current_callframe_mut()?, &mut next_index);
            self.next_log_index = next_index;
        }
        // The top-level frame reports the transaction's total gas used, matching the
        // receipt (post-refund). geth does the same: `callstack[0].GasUsed = receipt.GasUsed`.
        // `ctx_result.gas_used` is the EIP-7778 block-accounting value on Amsterdam+ and the
        // pre-refund total elsewhere; `gas_spent` is always the post-refund amount the sender
        // pays. Inner frames keep their own consumed gas (`gas_used`).
        let gas_used = if is_top_call {
            ctx_result.gas_spent
        } else {
            ctx_result.gas_used
        };
        // geth's `processOutput`: a successful frame carries the return data; a REVERT
        // with data exposes the data and decodes any `Error(string)` revert reason; any
        // other failure (exceptional halt / empty revert) carries neither.
        let output = ctx_result.output.clone();
        let (output, error, revert_reason) = match ctx_result.result {
            TxResult::Revert(ref err) => {
                let error = Some(geth_error_string(err));
                if err.is_revert_opcode() && !output.is_empty() {
                    (output.clone(), error, decode_revert_reason(&output))
                } else {
                    (Bytes::new(), error, None)
                }
            }
            _ => (output, None, None),
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
        // Early-out reasons are internal tokens (e.g. "OutOfFund"); normalize to the
        // geth-compatible error string the callTracer reports.
        let error = error.map(|token| geth_error_from_token(&token));
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
            // Placeholder: the block-absolute index is assigned in `assign_log_indices`
            // after reverted logs are pruned, so surviving logs stay gap-free like geth.
            index: 0,
            position: match callframe.calls.len().try_into() {
                Ok(pos) => pos,
                Err(_) => return Err(InternalError::TypeConversion),
            },
        };

        callframe.logs.push(log);
        Ok(())
    }

    fn current_callframe_mut(&mut self) -> Result<&mut CallTraceFrame, InternalError> {
        self.callframes.last_mut().ok_or(InternalError::CallFrame)
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
    // geth drops `to` on a failed CREATE/CREATE2 (no contract was deployed).
    if error.is_some() && matches!(callframe.call_type, CallType::CREATE | CallType::CREATE2) {
        callframe.to = None;
    }
    callframe.error = error;
    callframe.revert_reason = revert_reason;
}

/// Maps a LEVM [`VMError`] to the error string geth's callTracer emits (from
/// `core/vm/errors.go`).
///
/// The static errors match geth byte-for-byte. `stack underflow`, `stack overflow`
/// and `invalid opcode` are geth-formatted with operands (`stack underflow (1 <=> 2)`,
/// `invalid opcode: STOP`) that LEVM's unit variants don't carry, so we emit geth's
/// base wording for those. Variants without a geth analogue keep LEVM's own message.
fn geth_error_string(err: &VMError) -> String {
    use crate::errors::ExceptionalHalt::*;
    let mapped = match err {
        VMError::RevertOpcode => "execution reverted",
        VMError::ExceptionalHalt(halt) => match halt {
            OutOfGas => "out of gas",
            InvalidJump => "invalid jump destination",
            OpcodeNotAllowedInStaticContext => "write protection",
            AddressAlreadyOccupied => "contract address collision",
            ContractOutputTooBig => "max code size exceeded",
            InvalidContractPrefix => "invalid code: must not begin with 0xef",
            // geth formats these with operands we don't carry — emit its base wording.
            StackUnderflow => "stack underflow",
            StackOverflow => "stack overflow",
            InvalidOpcode => "invalid opcode",
            // No direct geth analogue; keep LEVM's message.
            VeryLargeNumber | OutOfBounds | Precompile(_) => return err.to_string(),
        },
        _ => return err.to_string(),
    };
    mapped.to_string()
}

/// Maps a CALL/CREATE early-out token (passed to [`LevmCallTracer::exit_early`]) to
/// the geth-compatible error string the callTracer reports.
fn geth_error_from_token(token: &str) -> String {
    match token {
        "OutOfFund" => "insufficient balance for transfer",
        "MaxDepth" => "max call depth exceeded",
        "MaxNonce" => "nonce uint64 overflow",
        "CreateAccExists" => "contract address collision",
        other => return other.to_string(),
    }
    .to_string()
}

/// Decodes an ABI-encoded `Error(string)` revert payload into its message, mirroring
/// geth's `abi.UnpackRevert`. Returns `None` for any payload that isn't a canonical
/// `Error(string)` (e.g. `Panic(uint256)` or custom errors), matching geth.
fn decode_revert_reason(output: &[u8]) -> Option<String> {
    // selector("Error(string)") = 0x08c379a0
    const SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];
    if output.get(..4)? != SELECTOR {
        return None;
    }
    let word_as_usize = |start: usize| -> Option<usize> {
        let word = output.get(start..start.checked_add(32)?)?;
        usize::try_from(U256::from_big_endian(word)).ok()
    };
    // head: offset to the string's dynamic-data region (relative to the args block,
    // i.e. after the 4-byte selector).
    let len_start = 4usize.checked_add(word_as_usize(4)?)?;
    let len = word_as_usize(len_start)?;
    let data_start = len_start.checked_add(32)?;
    let data = output.get(data_start..data_start.checked_add(len)?)?;
    String::from_utf8(data.to_vec()).ok()
}

/// Assigns each surviving log its block-absolute `index` (geth's `log.Index`) by walking
/// the frame tree in emission order and counting from `*next`. A log with `position == p`
/// was emitted after `p` subcalls completed, so within a frame the order is: logs at
/// position 0, subcall 0, logs at position 1, subcall 1, …, trailing logs. Because
/// reverted logs were already pruned, the survivors get contiguous, gap-free indices —
/// matching geth, which decrements `logSize` on revert.
fn assign_log_indices(callframe: &mut CallTraceFrame, next: &mut u64) {
    let mut logs = callframe.logs.iter_mut().peekable();
    for (call_idx, subcall) in callframe.calls.iter_mut().enumerate() {
        let call_idx = u64::try_from(call_idx).unwrap_or(u64::MAX);
        while let Some(log) = logs.next_if(|l| l.position == call_idx) {
            log.index = *next;
            *next = next.saturating_add(1);
        }
        assign_log_indices(subcall, next);
    }
    for log in logs {
        log.index = *next;
        *next = next.saturating_add(1);
    }
}

/// Clears logs of any frame that failed *or* whose ancestor failed, since a reverted
/// frame rolls back its whole subtree's state (including subcall logs). Mirrors geth's
/// `clearFailedLogs`, which threads a `parentFailed` flag down the tree.
fn clear_reverted_logs(callframe: &mut CallTraceFrame, parent_failed: bool) {
    let failed = parent_failed || callframe.error.is_some();
    if failed {
        callframe.logs.clear();
    }
    for subcall in &mut callframe.calls {
        clear_reverted_logs(subcall, failed);
    }
}

impl<'a> VM<'a> {
    /// This method is intended to be accessed after transaction execution
    pub fn get_trace_result(&mut self) -> Result<CallTraceFrame, VMError> {
        self.tracer
            .callframes
            .pop()
            .ok_or(InternalError::CallFrame.into())
    }
}
