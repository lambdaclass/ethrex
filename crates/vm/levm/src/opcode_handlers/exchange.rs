use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};

// Exchange Operations (16)
// Opcodes: SWAP1 ... SWAP16

impl<'a> VM<'a> {
    // SWAP operation
    pub fn op_swap<const N: usize>(&mut self) -> Result<OpcodeResult, VMError> {
        let cur_frame = self.cur_frame_mut()?;
        cur_frame.increase_consumed_gas(gas_cost::SWAPN)?;

        if cur_frame.stack.len() < N {
            return Err(ExceptionalHalt::StackUnderflow.into());
        }
        cur_frame.stack.swap(N)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
