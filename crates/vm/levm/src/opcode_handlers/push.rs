use crate::{
    call_frame::CallFrame,
    constants::WORD_SIZE,
    errors::{InternalError, OpcodeSuccess, VMError},
    gas_cost,
    opcodes::Opcode,
    vm::VM,
};
use ethrex_core::U256;

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

        let n_bytes = (usize::from(op))
            .checked_sub(usize::from(Opcode::PUSH1))
            .ok_or(VMError::InvalidOpcode)?
            .checked_add(1)
            .ok_or(VMError::InvalidOpcode)?;

        let read_n_bytes: Vec<u8> = current_call_frame
            .bytecode
            .get(current_call_frame.pc()..)
            .unwrap_or_default()
            .iter()
            .take(n_bytes)
            .cloned()
            .collect();

        let mut value_to_push = [0u8; WORD_SIZE];

        let start_index = WORD_SIZE.checked_sub(n_bytes).ok_or(VMError::Internal(
            InternalError::ArithmeticOperationUnderflow,
        ))?;

        // Rellenamos el array `value_to_push` con los bytes leídos
        for (i, byte) in read_n_bytes.iter().enumerate() {
            let index = start_index.checked_add(i).ok_or(VMError::Internal(
                InternalError::ArithmeticOperationOverflow,
            ))?;
            if let Some(data_byte) = value_to_push.get_mut(index) {
                *data_byte = *byte;
            }
        }

        let bytes_push: &[u8] = &value_to_push;
        current_call_frame.stack.push(U256::from(bytes_push))?;

        current_call_frame.increment_pc_by(n_bytes)?;

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
