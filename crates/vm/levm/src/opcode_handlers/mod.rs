use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    vm::VM,
};

pub mod arithmetic;
pub mod bitwise_comparison;
pub mod block;
pub mod dup;
pub mod environment;
pub mod exchange;
pub mod keccak;
pub mod logging;
pub mod push;
pub mod stack_memory_storage_flow;
pub mod system;

pub trait OpcodeHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError>;
}

pub struct OpStopHandler;
impl OpcodeHandler for OpStopHandler {
    fn eval(_vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        Ok(OpcodeResult::Halt)
    }
}

pub struct OpInvalidHandler;
impl OpcodeHandler for OpInvalidHandler {
    fn eval(_vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        Err(ExceptionalHalt::InvalidOpcode.into())
    }
}
