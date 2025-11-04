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
        self.current_call_frame
            .increase_consumed_gas(gas_cost::SWAPN)?;

        if self.current_stack().len() < N {
            return Err(ExceptionalHalt::StackUnderflow.into());
        }
        self.current_stack().swap(N)?;

        Ok(OpcodeResult::Continue)
    }
}
