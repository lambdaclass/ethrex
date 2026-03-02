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
    #[inline]
    pub fn op_push<const N: usize>(&mut self) -> Result<OpcodeResult, VMError> {
        let call_frame = &mut self.current_call_frame;
        call_frame.increase_consumed_gas(gas_cost::PUSHN)?;

        // Check to avoid multiple checks.
        let Some(new_pc) = call_frame.pc.checked_add(N) else {
            return Err(InternalError::Overflow.into());
        };

        let value = if let Some(slice) = call_frame.bytecode.bytecode.get(call_frame.pc..new_pc) {
            u256_from_big_endian_const(
                // SAFETY: If the get succeeded, we got N elements so the cast is safe.
                #[expect(unsafe_code)]
                unsafe {
                    *slice.as_ptr().cast::<[u8; N]>()
                },
            )
        } else {
            // NOTE: this isn't exactly correct, since a PUSHN with insufficient bytes should pad with zeros,
            // but if we're out of bytes, the next instruction will halt, discarding the stack anyway.
            U256::zero()
        };

        call_frame.stack.push(value)?;

        // Advance the PC by the number of bytes in this instruction's payload.
        call_frame.pc = new_pc;

        Ok(OpcodeResult::Continue)
    }

    // PUSH0
    #[inline]
    pub fn op_push0(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::PUSH0)?;
        self.current_call_frame.stack.push_zero()?;
        Ok(OpcodeResult::Continue)
    }
}
