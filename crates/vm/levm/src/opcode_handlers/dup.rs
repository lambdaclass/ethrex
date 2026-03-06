//! # Stack duplication operations
//!
//! Includes the following opcodes:
//!   - `DUP1` to `DUP16`

use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    gas_cost,
    opcode_handlers::OpcodeHandler,
    vm::VM,
};

/// Implementation for the `DUPn` opcodes.
pub struct OpDupHandler<const N: usize>;
impl<const N: usize> OpcodeHandler for OpDupHandler<N> {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::DUPN)?;

        vm.current_call_frame.stack.dup::<N>()?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `DUPN` opcode.
pub struct OpDupNHandler;
impl OpcodeHandler for OpDupNHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::DUPN)?;

        let relative_offset = vm
            .current_call_frame
            .bytecode
            .bytecode
            .get(vm.current_call_frame.pc)
            .copied()
            .unwrap_or_default();

        // Remove offsets that break backwards compatibility, which are
        //   - 0x5B, which corresponds to a JUMPDEST opcode.
        //   - 0x5F to 0x7F, which corresponds to PUSHx opcodes.
        //   - The extra 3 values (0x5C, 0x5D and 0x5E) are probably included to simplify decoding.
        let relative_offset = match relative_offset {
            x if x <= 0x5A => x.wrapping_add(17),
            x if x < 0x80 => return Err(ExceptionalHalt::InvalidOpcode.into()),
            x => x.wrapping_sub(20),
        };

        // Stack grows downwards, so we add the offset to get deeper elements
        // relative_offset is 1-indexed stack depth (17-235), convert to 0-indexed for array access
        // The n-th element (1-indexed) is at array index offset + (n-1)
        let absolute_offset = vm
            .current_call_frame
            .stack
            .offset
            .checked_add(usize::from(relative_offset).wrapping_sub(1))
            .ok_or(ExceptionalHalt::StackUnderflow)?;

        // Verify the offset is within stack bounds
        if absolute_offset >= vm.current_call_frame.stack.values.len() {
            return Err(ExceptionalHalt::StackUnderflow.into());
        }

        #[expect(unsafe_code, reason = "bound already checked")]
        vm.current_call_frame.stack.push(unsafe {
            *vm.current_call_frame
                .stack
                .values
                .get_unchecked(absolute_offset)
        })?;

        vm.current_call_frame.pc = vm.current_call_frame.pc.wrapping_add(1);
        Ok(OpcodeResult::Continue)
    }
}
