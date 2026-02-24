use crate::{
    constants::STACK_LIMIT,
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    vm::VM,
};
use std::mem;

// Exchange Operations (16)
// Opcodes: SWAP1 ... SWAP16

impl<'a> VM<'a> {
    // SWAP operation
    #[inline]
    pub fn op_swap<const N: usize>(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;

        current_call_frame.stack.swap::<N>()?;

        Ok(OpcodeResult::Continue)
    }

    // SWAPN operation
    #[inline]
    pub fn op_swapn(&mut self) -> Result<OpcodeResult, VMError> {
        let relative_offset = self
            .current_call_frame
            .bytecode
            .bytecode
            .get(self.current_call_frame.pc)
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
        // SWAPN swaps top with the (n+1)th element where n = decoded relative_offset
        // The (n+1)th element (1-indexed) is at array index offset + n
        let absolute_offset = self
            .current_call_frame
            .stack
            .offset
            .checked_add(usize::from(relative_offset))
            .ok_or(ExceptionalHalt::StackUnderflow)?;

        // Verify the offset is within stack bounds
        if absolute_offset >= STACK_LIMIT {
            return Err(ExceptionalHalt::StackUnderflow.into());
        }

        let top_offset = self.current_call_frame.stack.offset;

        #[expect(unsafe_code, reason = "bound already checked")]
        unsafe {
            let [x, y] = self
                .current_call_frame
                .stack
                .values
                .get_disjoint_unchecked_mut([top_offset, absolute_offset]);
            mem::swap(x, y);
        }

        self.current_call_frame.pc = self.current_call_frame.pc.wrapping_add(1);
        Ok(OpcodeResult::Continue)
    }

    // EXCHANGE operation
    #[inline]
    pub fn op_exchange(&mut self) -> Result<OpcodeResult, VMError> {
        let relative_offset = self
            .current_call_frame
            .bytecode
            .bytecode
            .get(self.current_call_frame.pc)
            .copied()
            .unwrap_or_default();

        // Remove offsets that break backwards compatibility, which are
        //   - 0x5B, which corresponds to a JUMPDEST opcode.
        //   - 0x5F to 0x7F, which corresponds to PUSHx opcodes.
        //   - The extra 3 values (0x5C, 0x5D and 0x5E) are probably included to simplify decoding.
        //
        // This range is more restricted than the one in DUPN and SWAPN because this payload
        // contains two values, and the decoded offsets would overlap. In other words, it avoids
        // having two different EXCHANGE encodings for the exact same offsets.
        let relative_offset = {
            let byte = match relative_offset {
                x if x <= 0x4F => x,
                x if x < 0x80 => return Err(ExceptionalHalt::InvalidOpcode.into()),
                x => x.wrapping_sub(48),
            };

            let q = byte >> 4;
            let r = byte & 0x0F;

            #[expect(
                clippy::arithmetic_side_effects,
                reason = "ranges are limited, cannot overflow or underflow"
            )]
            if q < r {
                (q + 1, r + 1)
            } else {
                (r + 1, 29 - q)
            }
        };

        // Stack grows downwards, so we add the offsets to get deeper elements
        let absolute_offset = {
            let stack_offset = self.current_call_frame.stack.offset;

            let q = stack_offset
                .checked_add(usize::from(relative_offset.0))
                .ok_or(ExceptionalHalt::StackUnderflow)?;
            let r = stack_offset
                .checked_add(usize::from(relative_offset.1))
                .ok_or(ExceptionalHalt::StackUnderflow)?;

            // Verify both offsets are within stack bounds
            if q >= STACK_LIMIT || r >= STACK_LIMIT {
                return Err(ExceptionalHalt::StackUnderflow.into());
            }

            (q, r)
        };

        #[expect(unsafe_code, reason = "bound already checked")]
        unsafe {
            let [x, y] = self
                .current_call_frame
                .stack
                .values
                .get_disjoint_unchecked_mut([absolute_offset.0, absolute_offset.1]);
            mem::swap(x, y);
        }

        self.current_call_frame.pc = self.current_call_frame.pc.wrapping_add(1);
        Ok(OpcodeResult::Continue)
    }
}
