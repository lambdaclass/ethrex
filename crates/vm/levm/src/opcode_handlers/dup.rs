use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
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

        // Ensure the stack has enough elements to duplicate
        if current_call_frame.stack.len() < depth {
            return Err(ExceptionalHalt::StackUnderflow.into());
        }

        // Get the value at the specified depth
        let value_at_depth = current_call_frame.stack.get(
            current_call_frame
                .stack
                .len()
                .checked_sub(depth)
                .ok_or(ExceptionalHalt::StackUnderflow)?,
        )?;

        // Push the duplicated value onto the stack
        current_call_frame.stack.push(*value_at_depth)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
