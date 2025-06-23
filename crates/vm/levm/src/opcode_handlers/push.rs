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

impl<'a> VM<'a> {
    // Generic PUSH operation
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

    /// Specialized PUSH1 operation
    ///
    /// We use specialized push1 and push2 implementations because they are way more frequent than the others,
    /// so their impact on performance is significant.
    /// These implementations allow using U256::from, which is considerable more performant than U256::from_big_endian)
    pub fn op_push1(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::PUSHN)?;

        let value = read_bytcode_slice_const::<1>(current_call_frame)?[0];

        current_call_frame.stack.push(U256::from(value))?;

        Ok(OpcodeResult::Continue {
            // The 1 byte that you push to the stack + 1 for the next instruction
            pc_increment: 2,
        })
    }

    // Specialized PUSH2 operation
    pub fn op_push2(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::PUSHN)?;

        let read_n_bytes = read_bytcode_slice_const::<2>(current_call_frame)?;

        let value = u16::from_be_bytes(read_n_bytes);

        current_call_frame.stack.push(U256::from(value))?;

        Ok(OpcodeResult::Continue {
            // The 2 bytes that you push to the stack + 1 for the next instruction
            pc_increment: 3,
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

// Like `read_bytcode_slice` but using a const generic and returning a fixed size array.
fn read_bytcode_slice_const<const N: usize>(
    current_call_frame: &CallFrame,
) -> Result<[u8; N], VMError> {
    let current_pc = current_call_frame.pc;
    let pc_offset = current_pc
        // Add 1 to the PC because we don't want to include the
        // Bytecode of the current instruction in the data we're about
        // to read. We only want to read the data _NEXT_ to that
        // bytecode
        .checked_add(1)
        .ok_or(InternalError::Overflow)?;

    if let Some(slice) = current_call_frame
        .bytecode
        .get(pc_offset..pc_offset.checked_add(N).ok_or(OutOfBounds)?)
    {
        Ok(slice
            .try_into()
            .map_err(|_| VMError::Internal(InternalError::TypeConversion))?)
    } else {
        Ok([0; N])
    }
}
