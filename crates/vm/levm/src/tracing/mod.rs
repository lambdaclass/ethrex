#![allow(unused_variables)]

mod call_tracer;

pub use call_tracer::*;

use bytes::Bytes;
use ethrex_common::tracing::CallType;
use ethrex_common::types::{Log, Transaction};
use ethrex_common::{Address, H256, U256};

use crate::Environment;
use crate::db::gen_db::GeneralizedDatabase;
use crate::errors::InternalError;
use crate::opcodes::Opcode;

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
        _env: &Environment,
        _tx: &Transaction,
        _from: Address,
        _db: &mut GeneralizedDatabase,
    ) {
    }

    /// Called after txn execution ends
    fn txn_end(&mut self, _gas_used: u64, err: Option<String>, _db: &mut GeneralizedDatabase) {}

    /// Called before each opcode execution. Used by prestate tracer to capture account lookups.
    /// Returns true if tracing should continue, false to interrupt execution.
    fn on_opcode(
        &mut self,
        _opcode: Opcode,
        _current_address: Address,
        _stack: &[U256],
        _db: &mut GeneralizedDatabase,
    ) -> bool {
        true
    }

    /// Called when a storage slot is accessed (SLOAD/SSTORE).
    fn on_storage_access(&mut self, _address: Address, _slot: H256, _db: &mut GeneralizedDatabase) {
    }

    /// Called when an account is accessed (BALANCE, EXTCODESIZE, EXTCODEHASH, EXTCODECOPY).
    fn on_account_access(&mut self, _address: Address, _db: &mut GeneralizedDatabase) {}

    /// Called when a contract is created (CREATE/CREATE2).
    fn on_create(&mut self, _address: Address, _db: &mut GeneralizedDatabase) {}

    /// Called when SELFDESTRUCT is executed.
    fn on_selfdestruct(&mut self, _address: Address, _db: &mut GeneralizedDatabase) {}
}

pub struct NoOpTracer;

impl Tracer for NoOpTracer {}
