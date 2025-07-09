use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};
use ethrex_common::{U256, types::Fork, utils::u256_from_big_endian_const};

// Push Operations
// Opcodes: PUSH0, PUSH1 ... PUSH32

impl<'a> VM<'a> {
    // Generic PUSH operation, optimized at compile time for the given N.
    pub fn op_push<const N: usize>(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::PUSHN)?;

        let pc_offset = current_call_frame
            .pc
            // Add 1 to the PC because we don't want to include the
            // Bytecode of the current instruction in the data we're about
            // to read. We only want to read the data _NEXT_ to that
            // bytecode
            .wrapping_add(1);

        let end = pc_offset.wrapping_add(N);

        let value = if end <= current_call_frame.bytecode.len() {
            #[allow(clippy::indexing_slicing, clippy::expect_used)]
            u256_from_big_endian_const::<N>(
                current_call_frame.bytecode[pc_offset..pc_offset.wrapping_add(N)]
                    .try_into()
                    .expect("shoud not"),
            )
        } else {
            U256::zero()
        };

        current_call_frame.stack.push(&[value])?;

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
