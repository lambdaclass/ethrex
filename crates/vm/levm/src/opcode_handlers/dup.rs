use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    vm::VM,
};

// Duplication Operation (16)
// Opcodes: DUP1 ... DUP16

impl<'a> VM<'a> {
    // DUP operation
    #[inline]
    pub fn op_dup<const N: usize>(&mut self) -> Result<OpcodeResult, VMError> {
        // Duplicate the value at the specified depth
        self.current_call_frame.stack.dup::<N>()?;

        Ok(OpcodeResult::Continue)
    }

    // DUPN operation
    #[inline]
    pub fn op_dupn(&mut self) -> Result<OpcodeResult, VMError> {
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
        // relative_offset is 1-indexed stack depth (17-235), convert to 0-indexed for array access
        // The n-th element (1-indexed) is at array index offset + (n-1)
        let absolute_offset = self
            .current_call_frame
            .stack
            .offset
            .checked_add(usize::from(relative_offset).wrapping_sub(1))
            .ok_or(ExceptionalHalt::StackUnderflow)?;

        // Verify the offset is within stack bounds
        if absolute_offset >= self.current_call_frame.stack.values.len() {
            return Err(ExceptionalHalt::StackUnderflow.into());
        }

        #[expect(unsafe_code, reason = "bound already checked")]
        self.current_call_frame.stack.push(unsafe {
            *self
                .current_call_frame
                .stack
                .values
                .get_unchecked(absolute_offset)
        })?;

        self.current_call_frame.pc = self.current_call_frame.pc.wrapping_add(1);
        Ok(OpcodeResult::Continue)
    }
}
