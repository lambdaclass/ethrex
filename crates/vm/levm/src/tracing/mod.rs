#![allow(unused_variables)]

mod call_tracer;

pub use call_tracer::*;

use bytes::Bytes;
use ethrex_common::tracing::CallType;
use ethrex_common::types::Log;
use ethrex_common::{Address, U256};

use crate::errors::{ContextResult, InternalError};

pub trait Tracer {
    fn enter(
        &mut self,
        _call_type: CallType,
        _from: Address,
        _to: Address,
        _value: U256,
        _gas: u64,
        _input: &Bytes,
    ) {
    }

    fn exit(
        &mut self,
        _gas_used: u64,
        _output: Bytes,
        _error: Option<String>,
        _revert_reason: Option<String>,
    ) -> Result<(), InternalError> {
        Ok(())
    }

    /// Exits trace call using the ContextResult.
    fn exit_context(
        &mut self,
        _ctx_result: &ContextResult,
        _is_top_call: bool,
    ) -> Result<(), InternalError> {
        Ok(())
    }

    /// Exits trace call when CALL or CREATE opcodes return early or in case SELFDESTRUCT is called.
    fn exit_early(&mut self, _gas_used: u64, _error: Option<String>) -> Result<(), InternalError> {
        Ok(())
    }

    /// Registers log when opcode log is executed.
    fn log(&mut self, _log: &Log) -> Result<(), InternalError> {
        Ok(())
    }
}

pub struct NoOpTracer;

impl Tracer for NoOpTracer {}
