#![allow(unused_variables)]

mod call_tracer;

pub use call_tracer::*;

use bytes::Bytes;
use ethrex_common::tracing::CallType;
use ethrex_common::types::{Log, Transaction};
use ethrex_common::{Address, U256};

use crate::Environment;
use crate::db::gen_db::GeneralizedDatabase;
use crate::errors::InternalError;

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
        _depth: usize,
        _gas_used: u64,
        _output: Bytes,
        _error: Option<String>,
        _revert_reason: Option<String>,
    ) -> Result<(), InternalError> {
        Ok(())
    }

    /// Registers log when opcode log is executed.
    fn log(&mut self, _log: &Log) -> Result<(), InternalError> {
        Ok(())
    }

    /// Called before txn execution starts
    fn txn_start(
        &mut self,
        env: &Environment,
        tx: &Transaction,
        from: Address,
        db: &mut GeneralizedDatabase,
    ) {
    }

    /// Called after txn execution starts
    fn txn_end(&mut self, gas_used: u64, db: &mut GeneralizedDatabase) {}
}

pub struct NoOpTracer;

impl Tracer for NoOpTracer {}
