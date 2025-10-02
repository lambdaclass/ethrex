use std::cell::OnceCell;

use crate::{
    errors::{OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};

// Duplication Operation (16)
// Opcodes: DUP1 ... DUP16

impl<'a> VM<'a> {
    // DUP operation
    pub fn op_dup<const N: usize>(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        // Increase the consumed gas
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::DUPN)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        // Get the value at the specified depth
        let value_at_depth = match self.current_call_frame.stack.get(N) {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        // Push the duplicated value onto the stack
        if let Err(err) = self.current_call_frame.stack.push1(value_at_depth) {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        OpcodeResult::Continue
    }
}
