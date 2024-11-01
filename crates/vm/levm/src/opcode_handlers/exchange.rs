use crate::{
    call_frame::CallFrame,
    constants::gas_cost,
    errors::{OpcodeSuccess, VMError},
    opcodes::Opcode,
    vm::VM,
};

// Exchange Operations (16)
// Opcodes: SWAP1 ... SWAP16

impl VM {
    // SWAP operation
    pub fn op_swap(
        &mut self,
        current_call_frame: &mut CallFrame,
        op: Opcode,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::SWAPN)?;

        let depth = (op as u8) - (Opcode::SWAP1 as u8) + 1;

        if current_call_frame.stack.len() < depth as usize {
            return Err(VMError::StackUnderflow);
        }
        let stack_top_index = current_call_frame.stack.len();
        let to_swap_index = stack_top_index
            .checked_sub(depth as usize)
            .ok_or(VMError::StackUnderflow)?;
        current_call_frame
            .stack
            .swap(stack_top_index - 1, to_swap_index - 1);

        Ok(OpcodeSuccess::Continue)
    }
}
