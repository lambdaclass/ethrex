use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};

// Exchange Operations (16)
// Opcodes: SWAP1 ... SWAP16

impl<'a> VM<'a> {
    // SWAP operation
    pub fn op_swap(&mut self, depth: usize) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::SWAPN)?;

        let stack_top_index = current_call_frame
            .stack
            .len()
            .checked_sub(1)
            .ok_or(ExceptionalHalt::StackUnderflow)?;

        if current_call_frame.stack.len() < depth {
            return Err(ExceptionalHalt::StackUnderflow.into());
        }
        let to_swap_index = stack_top_index
            .checked_sub(depth)
            .ok_or(ExceptionalHalt::StackUnderflow)?;
        current_call_frame
            .stack
            .swap(stack_top_index, to_swap_index)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
