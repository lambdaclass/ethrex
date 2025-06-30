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

        if self.stack.len() < depth {
            return Err(ExceptionalHalt::StackUnderflow.into());
        }
        self.stack.swap(depth)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
