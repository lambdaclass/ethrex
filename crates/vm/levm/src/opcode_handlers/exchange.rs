use std::cell::OnceCell;

use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};

// Exchange Operations (16)
// Opcodes: SWAP1 ... SWAP16

impl<'a> VM<'a> {
    // SWAP operation
    pub fn op_swap<const N: usize>(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::SWAPN)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if self.current_call_frame.stack.len() < N {
            error.set(ExceptionalHalt::StackUnderflow.into());
            return OpcodeResult::Halt;
        }
        if let Err(err) = self.current_call_frame.stack.swap(N) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }
}
