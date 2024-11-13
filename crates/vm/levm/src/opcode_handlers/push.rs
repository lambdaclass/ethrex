use crate::{
    call_frame::CallFrame,
    constants::gas_cost,
    errors::{OpcodeSuccess, VMError},
    opcodes::Opcode,
    vm::VM,
};
use ethereum_rust_core::U256;

// Push Operations
// Opcodes: PUSH0, PUSH1 ... PUSH32

impl VM {
    // PUSH operation
    pub fn op_push(
        &mut self,
        current_call_frame: &mut CallFrame,
        op: Opcode,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::PUSHN)?;

        let n_bytes = usize::from(op) - usize::from(Opcode::PUSH1) + 1;

        let next_n_bytes = current_call_frame
            .bytecode
            .get(current_call_frame.pc()..current_call_frame.pc() + n_bytes)
            .ok_or(VMError::InvalidBytecode)?; // This shouldn't happen during execution

        let value_to_push = U256::from(next_n_bytes);

        current_call_frame.stack.push(value_to_push)?;

        current_call_frame.increment_pc_by(n_bytes);

        Ok(OpcodeSuccess::Continue)
    }

    // PUSH0
    pub fn op_push0(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::PUSH0)?;

        current_call_frame.stack.push(U256::zero())?;

        Ok(OpcodeSuccess::Continue)
    }
}
