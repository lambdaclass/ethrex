use std::cell::OnceCell;

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
    pub fn op_push<const N: usize>(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::PUSHN)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        // Check to avoid multiple checks.
        if self.current_call_frame.pc.checked_add(N).is_none() {
            error.set(InternalError::Overflow.into());
            return OpcodeResult::Halt;
        }

        let value = if let Some(slice) = self
            .current_call_frame
            .bytecode
            .get(self.current_call_frame.pc..self.current_call_frame.pc.wrapping_add(N))
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

        if let Err(err) = self.current_call_frame.stack.push1(value) {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        // Advance the PC by the number of bytes in this instruction's payload.
        self.current_call_frame.pc = self.current_call_frame.pc.wrapping_add(N);

        OpcodeResult::Continue
    }

    // PUSH0
    pub fn op_push0(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::PUSH0)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        if let Err(err) = self.current_call_frame.stack.push_zero() {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }
}
