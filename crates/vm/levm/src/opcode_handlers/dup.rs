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

        // Get the value at the specified depth
        let value_at_depth = *self.current_stack().get(N)?;

        // Push the duplicated value onto the stack
        self.current_stack().push1(value_at_depth)?;

        Ok(OpcodeResult::Continue)
    }
}
