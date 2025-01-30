use crate::{
    call_frame::CallFrame,
    errors::{OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};

// Exchange Operations (16)
// Opcodes: SWAP1 ... SWAP16

impl VM {
    // SWAP operation
    pub fn op_swap(
        &mut self,
        current_call_frame: &mut CallFrame,
        depth: usize,
    ) -> Result<OpcodeResult, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::SWAPN)?;

        let stack_top_index = current_call_frame
            .stack
            .len()
            .checked_sub(1)
            .ok_or(VMError::StackUnderflow)?;

        if current_call_frame.stack.len() < depth {
            return Err(VMError::StackUnderflow);
        }
        let to_swap_index = stack_top_index
            .checked_sub(depth)
            .ok_or(VMError::StackUnderflow)?;
        current_call_frame
            .stack
            .swap(stack_top_index, to_swap_index)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
