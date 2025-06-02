use bytes::Bytes;
use ethrex_common::{types::Log, Address, U256};
use keccak_hash::H256;
use serde::Serialize;

use crate::{
    errors::{ExecutionReport, InternalError, TxResult},
    opcodes::Opcode,
};

#[derive(Debug, Default)]
/// Geth's callTracer (https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers)
/// Use `LevmCallTracer::disabled()` when tracing is not wanted.
pub struct LevmCallTracer {
    /// Stack for tracer callframes, at the end of execution there will be only one element.
    pub callframes: Vec<TracerCallFrame>,
    /// If true, trace only the top call (a.k.a. the external transaction)
    pub only_top_call: bool,
    /// If true, trace logs
    pub with_log: bool,
    /// If active is set to false it won't trace.
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TracerCallFrame {
    #[serde(rename = "type")]
    pub call_type: Opcode,
    pub from: Address,
    pub to: Address,
    #[serde(serialize_with = "to_hex")]
    pub value: U256,
    #[serde(serialize_with = "to_hex")]
    pub gas: u64,
    #[serde(rename = "gasUsed", serialize_with = "to_hex")]
    pub gas_used: u64,
    #[serde(serialize_with = "to_hex")]
    pub input: Bytes,
    #[serde(serialize_with = "to_hex")]
    pub output: Bytes,
    #[serde(serialize_with = "option_string_empty_as_str")]
    pub error: Option<String>,
    #[serde(rename = "revertReason", serialize_with = "option_string_empty_as_str")]
    pub revert_reason: Option<String>,
    pub calls: Vec<TracerCallFrame>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub logs: Vec<TracerLog>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TracerLog {
    pub address: Address,
    pub topics: Vec<H256>,
    #[serde(serialize_with = "to_hex")]
    pub data: Bytes,
    #[serde(serialize_with = "to_hex")]
    pub position: usize,
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
    /// (For now that we only implement one tracer is the most convenient solution.
    /// In the future a NoOpTracer may be more convenient)
    pub fn disabled() -> Self {
        LevmCallTracer {
            active: false,
            ..Default::default()
        }
    }

    /// Starts trace call.
    pub fn enter(
        &mut self,
        call_type: Opcode,
        from: Address,
        to: Address,
        value: U256,
        gas: u64,
        input: Bytes,
    ) {
        if !self.active {
            return;
        }
        if self.only_top_call && !self.callframes.is_empty() {
            // Only create callframe if it's the first one to be created.
            return;
        }
        let callframe = TracerCallFrame::new(call_type, from, to, value, gas, input);
        self.callframes.push(callframe);
    }

    /// Exits trace call.
    fn exit(
        &mut self,
        gas_used: u64,
        output: Bytes,
        error: Option<String>,
        revert_reason: Option<String>,
    ) -> Result<(), InternalError> {
        let mut executed_callframe = self
            .callframes
            .pop()
            .ok_or(InternalError::CouldNotPopCallframe)?;

        executed_callframe.process_output(gas_used, output, error, revert_reason);

        // Append executed callframe to parent callframe if appropriate.
        if let Some(parent_callframe) = self.callframes.last_mut() {
            parent_callframe.calls.push(executed_callframe);
        } else {
            self.callframes.push(executed_callframe);
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
            // After finishing transaction execution clear all logs that reverted.
            self.callframes
                .last_mut()
                .ok_or(InternalError::CouldNotAccessLastCallframe)?
                .clear_reverted_logs();
        }
        let (gas_used, output) = (report.gas_used, report.output.clone());

        let (error, revert_reason) = if let TxResult::Revert(ref err) = report.result {
            let reason = String::from_utf8(report.output.to_vec()).ok();
            (Some(err.to_string()), reason)
        } else {
            (None, None)
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

    pub fn log(&mut self, log: Log) -> Result<(), InternalError> {
        if !self.active || !self.with_log {
            return Ok(());
        }
        if self.only_top_call && self.callframes.len() > 1 {
            // Register logs for top call only.
            return Ok(());
        }
        let callframe = self
            .callframes
            .last_mut()
            .ok_or(InternalError::CouldNotAccessLastCallframe)?;

        let log = TracerLog {
            address: log.address,
            topics: log.topics,
            data: log.data,
            position: callframe.calls.len(),
        };

        callframe.logs.push(log);
        Ok(())
    }
}

impl TracerCallFrame {
    pub fn new(
        call_type: Opcode,
        from: Address,
        to: Address,
        value: U256,
        gas: u64,
        input: Bytes,
    ) -> Self {
        Self {
            call_type,
            from,
            to,
            value,
            gas,
            gas_used: 0,
            input,
            output: Bytes::new(),
            error: None,
            revert_reason: None,
            calls: Vec::new(),
            logs: Vec::new(),
        }
    }

    pub fn process_output(
        &mut self,
        gas_used: u64,
        output: Bytes,
        error: Option<String>,
        revert_reason: Option<String>,
    ) {
        self.gas_used = gas_used;
        self.output = output;
        self.error = error;
        self.revert_reason = revert_reason;
    }

    /// Clear logs from callframe if it reverted and repeat with its subcalls.
    pub fn clear_reverted_logs(&mut self) {
        if self.error.is_some() {
            self.logs.clear();
            for subcall in &mut self.calls {
                subcall.clear_reverted_logs();
            }
        }
    }
}

fn to_hex<T, S>(x: &T, s: S) -> Result<S::Ok, S::Error>
where
    T: std::fmt::LowerHex,
    S: serde::Serializer,
{
    s.serialize_str(&format!("0x{:x}", x))
}

fn option_string_empty_as_str<S>(x: &Option<String>, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_str(x.as_deref().unwrap_or(""))
}
