use crate::{
    errors::{OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};

// Duplication Operation (16)
// Opcodes: DUP1 ... DUP16

impl<'a> VM<'a> {
    // DUP operation
    pub fn op_dup<const N: usize>(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        // Increase the consumed gas
        current_call_frame.increase_consumed_gas(gas_cost::DUPN)?;

        // Get the value at the specified depth
        let value_at_depth = *current_call_frame.stack.get(N)?;

        // Push the duplicated value onto the stack
        current_call_frame.stack.push1(value_at_depth)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
