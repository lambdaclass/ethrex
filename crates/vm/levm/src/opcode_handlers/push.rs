use crate::{
    errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};
use ExceptionalHalt::OutOfBounds;
use ethrex_common::{U256, types::Fork, utils::u256_from_big_endian_const};

// Push Operations
// Opcodes: PUSH0, PUSH1 ... PUSH32

impl<'a> VM<'a> {
    // Generic PUSH operation, optimized at compile time for the given N.
    pub fn op_push<const N: usize>(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::PUSHN)?;

        // Do bounds checks
        let pc_offset = current_call_frame
            .pc
            .checked_add(1)
            .ok_or(InternalError::Overflow)?;

        let pc_offset_end = pc_offset.checked_add(N).ok_or(OutOfBounds)?;

        if current_call_frame.bytecode.len() >= pc_offset_end {
            // SAFETY: bounds have been checked beforehand.
            #[allow(unsafe_code)]
            let bytes = unsafe {
                *current_call_frame
                    .bytecode
                    .get_unchecked(pc_offset..pc_offset_end)
                    .first_chunk::<N>()
                    .unwrap_unchecked()
            };

            let value = u256_from_big_endian_const(bytes);
            current_call_frame.stack.push(&[value])?;
        } else {
            current_call_frame.stack.push(&[U256::zero()])?;
        }

        // The n_bytes that you push to the stack + 1 for the next instruction
        let increment_pc_by = N.wrapping_add(1);

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

        current_call_frame.stack.push(&[U256::zero()])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
