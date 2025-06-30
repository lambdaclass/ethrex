use crate::{
    errors::{OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};

// Duplication Operation (16)
// Opcodes: DUP1 ... DUP16

impl<'a> VM<'a> {
    // DUP operation
    pub fn op_dup(&mut self, depth: usize) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        // Increase the consumed gas
        current_call_frame.increase_consumed_gas(gas_cost::DUPN)?;

        // Get the value at the specified depth
        let value_at_depth = *self.stack.get(depth)?;

        // Push the duplicated value onto the stack
        self.stack.push(&[value_at_depth])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
