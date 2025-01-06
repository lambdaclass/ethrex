use crate::{
    call_frame::CallFrame,
    constants::WORD_SIZE,
    errors::{InternalError, OpcodeSuccess, VMError},
    gas_cost,
    vm::VM,
};
use ethrex_core::U256;
use revm_primitives::SpecId;

// Push Operations
// Opcodes: PUSH0, PUSH1 ... PUSH32

impl VM {
    // PUSH operation
    pub fn op_push(
        &mut self,
        current_call_frame: &mut CallFrame,
        n_bytes: usize,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::PUSHN)?;

        let read_n_bytes = read_bytcode_slice(current_call_frame, n_bytes)?;
        let value_to_push = bytes_to_word(read_n_bytes, n_bytes)?;

        current_call_frame
            .stack
            .push(U256::from_big_endian(value_to_push.as_slice()))?;

        current_call_frame.increment_pc_by(n_bytes)?;

        Ok(OpcodeSuccess::Continue)
    }

    // PUSH0
    pub fn op_push0(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        // [EIP-3855] - PUSH0 is only available from SHANGHAI
        if self.env.spec_id < SpecId::SHANGHAI {
            return Err(VMError::InvalidOpcode);
        }

        self.increase_consumed_gas(current_call_frame, gas_cost::PUSH0)?;

        current_call_frame.stack.push(U256::zero())?;

        Ok(OpcodeSuccess::Continue)
    }
}

fn read_bytcode_slice(current_call_frame: &CallFrame, n_bytes: usize) -> Result<&[u8], VMError> {
    let pc_offset = current_call_frame
        .pc()
        // Add 1 to the PC because we don't want to include the
        // Bytecode of the current instruction in the data we're about
        // to read. We only want to read the data _NEXT_ to that
        // bytecode
        .checked_add(1)
        .ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;

    Ok(current_call_frame
        .bytecode
        .get(pc_offset..pc_offset.checked_add(n_bytes).ok_or(VMError::OutOfBounds)?)
        .unwrap_or_default())
}

fn bytes_to_word(read_n_bytes: &[u8], n_bytes: usize) -> Result<[u8; WORD_SIZE], VMError> {
    let mut value_to_push = [0u8; WORD_SIZE];
    let start_index = WORD_SIZE.checked_sub(n_bytes).ok_or(VMError::Internal(
        InternalError::ArithmeticOperationUnderflow,
    ))?;

    for (i, byte) in read_n_bytes.iter().enumerate() {
        let index = start_index.checked_add(i).ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        if let Some(data_byte) = value_to_push.get_mut(index) {
            *data_byte = *byte;
        }
    }

    Ok(value_to_push)
}
