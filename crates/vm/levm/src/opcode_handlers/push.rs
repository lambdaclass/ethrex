use crate::{
    call_frame::CallFrame, errors::{ExceptionalHalt, InternalError, OpcodeResult, VMError}, gas_cost, vm::VM, TX
};
use ethrex_common::{U256, types::Fork, utils::u256_from_big_endian_const};
use tracing::info;

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

        let read_n_bytes = read_bytcode_slice::<N>(current_call_frame)?;
        let value = u256_from_big_endian_const(read_n_bytes);
        if *(TX.lock().unwrap()) {
            info!("PUSH-N, value for old implementation: {value}");
        }
        let value = if let Some(slice) = current_call_frame
            .bytecode
            .get(pc_offset..pc_offset.wrapping_add(N))
        {
            if *(TX.lock().unwrap()) {
                info!("PUSH-N, getting range {pc_offset}..{}, obtained slice: {slice:?}", pc_offset.wrapping_add(N));
                info!("PUSH-N, from big endian lib: {}", U256::from_big_endian(slice));
            }
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
        if *(TX.lock().unwrap()) {
            info!("PUSH-N: {value}");
        }

        current_call_frame.stack.push1(value)?;

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
        let current_call_frame = &mut self.current_call_frame;

        current_call_frame.increase_consumed_gas(gas_cost::PUSH0)?;

        current_call_frame.stack.push1(U256::zero())?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}

fn read_bytcode_slice<const N: usize>(current_call_frame: &CallFrame) -> Result<[u8; N], VMError> {
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
        .get(pc_offset..pc_offset.checked_add(N).unwrap())
    {
        Ok(slice
            .try_into()
            .map_err(|_| VMError::Internal(InternalError::TypeConversion))?)
    } else {
        Ok([0; N])
    }
}