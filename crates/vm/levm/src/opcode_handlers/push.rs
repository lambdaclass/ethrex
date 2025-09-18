use crate::{
    errors::{InternalError, OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};
use ethrex_common::{U256, utils::u256_from_big_endian_const};

// Push Operations
// Opcodes: PUSH0, PUSH1 ... PUSH32

impl<'a> VM<'a> {
    // Generic PUSH operation, optimized at compile time for the given N.
    pub fn op_push<const N: usize>(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::PUSHN)?;

        let current_pc = current_call_frame.pc;

        // Check to avoid multiple checks.
        if current_pc.checked_add(N.wrapping_add(1)).is_none() {
            Err(InternalError::Overflow)?;
        }

        let pc_offset = current_pc
            // Add 1 to the PC because we don't want to include the
            // Bytecode of the current instruction in the data we're about
            // to read. We only want to read the data _NEXT_ to that
            // bytecode
            .wrapping_add(1);

        let value = if let Some(slice) = current_call_frame
            .bytecode
            .get(pc_offset..pc_offset.wrapping_add(N))
        {
            u256_from_big_endian_const(
                // SAFETY: If the get succeeded, we got N elements so the cast is safe.
                #[expect(unsafe_code)]
                unsafe {
                    *slice.as_ptr().cast::<[u8; N]>()
                },
            )
        } else {
            U256::zero()
        };

        current_call_frame.stack.push1(value)?;

        // The n_bytes that you push to the stack + 1 for the next instruction
        let increment_pc_by = N.wrapping_add(1);

        Ok(OpcodeResult::Continue {
            pc_increment: increment_pc_by,
        })
    }

    // PUSH0
    pub fn op_push0(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;

        current_call_frame.increase_consumed_gas(gas_cost::PUSH0)?;

        current_call_frame.stack.push1(U256::zero())?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}
