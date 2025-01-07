use crate::{
    call_frame::CallFrame,
    errors::{OpcodeSuccess, VMError},
    gas_cost,
    vm::VM,
};

// Duplication Operation (16)
// Opcodes: DUP1 ... DUP16

impl VM {
    // DUP operation
    pub fn op_dup(
        &mut self,
        current_call_frame: &mut CallFrame,
        depth: usize,
    ) -> Result<OpcodeSuccess, VMError> {
        // Increase the consumed gas
        self.increase_consumed_gas(current_call_frame, gas_cost::DUPN)?;

        // Ensure the stack has enough elements to duplicate
        if current_call_frame.stack.len() < depth {
            return Err(VMError::StackUnderflow);
        }

        // Get the value at the specified depth
        let value_at_depth = current_call_frame.stack.get(
            current_call_frame
                .stack
                .len()
                .checked_sub(depth)
                .ok_or(VMError::StackUnderflow)?,
        )?;

        // Push the duplicated value onto the stack
        current_call_frame.stack.push(*value_at_depth)?;

        Ok(OpcodeSuccess::Continue)
    }
}
