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
pub mod frame_tx;
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
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        // The dispatch loop has already counted this byte, and only here is it
        // known that the fork doesn't define it. See `OpcodeCounter`.
        if let Some(counter) = vm.db.opcode_counter.as_deref_mut() {
            counter.discard_last();
        }
        Err(ExceptionalHalt::InvalidOpcode.into())
    }
}
