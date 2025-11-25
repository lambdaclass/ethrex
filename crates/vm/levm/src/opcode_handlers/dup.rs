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
        // Increase the consumed gas
        self.current_call_frame
            .increase_consumed_gas(gas_cost::DUPN)?;

        // Duplicate the value at the specified depth
        self.current_call_frame.stack.dup::<N>()?;

        Ok(OpcodeResult::Continue)
    }
}
