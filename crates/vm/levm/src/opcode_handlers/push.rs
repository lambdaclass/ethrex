use crate::{
    call_frame::CallFrame,
    errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};
use ExceptionalHalt::OutOfBounds;
use ethrex_common::{U256, types::Fork};

// Push Operations
// Opcodes: PUSH0, PUSH1 ... PUSH32

impl VM<'_> {
    // PUSH operation
    pub fn op_push(&mut self, n_bytes: usize) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::PUSHN)?;

        let read_n_bytes = read_bytcode_slice(current_call_frame, n_bytes)?;

        current_call_frame
            .stack
            .push(U256::from_big_endian(read_n_bytes))?;

        // The n_bytes that you push to the stack + 1 for the next instruction
        let increment_pc_by = n_bytes.wrapping_add(1);

        Ok(OpcodeResult::Continue {
            pc_increment: increment_pc_by,
        })
    }

    // PUSH0
    pub fn op_push0(&mut self) -> Result<OpcodeResult, VMError> {
        // [EIP-3855] - PUSH0 is only available from SHANGHAI
        if self.env.config.fork < Fork::Shanghai {
            return Err(ExceptionalHalt::InvalidOpcode.into());
        }
        let current_call_frame = self.current_call_frame_mut()?;

        current_call_frame.increase_consumed_gas(gas_cost::PUSH0)?;

        current_call_frame.stack.push(U256::zero())?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}

fn read_bytcode_slice(current_call_frame: &CallFrame, n_bytes: usize) -> Result<&[u8], VMError> {
    let current_pc = current_call_frame.pc;
    let pc_offset = current_pc
        // Add 1 to the PC because we don't want to include the
        // Bytecode of the current instruction in the data we're about
        // to read. We only want to read the data _NEXT_ to that
        // bytecode
        .checked_add(1)
        .ok_or(InternalError::Overflow)?;

    Ok(current_call_frame
        .bytecode
        .get(pc_offset..pc_offset.checked_add(n_bytes).ok_or(OutOfBounds)?)
        .unwrap_or_default())
}
