//! # Stack push operations
//!
//! Includes the following opcodes:
//!   - `PUSH0`
//!   - `PUSH1` to `PUSH32`

use crate::{
    errors::{InternalError, OpcodeResult, VMError},
    gas_cost,
    opcode_handlers::OpcodeHandler,
    vm::VM,
};
use ethrex_common::U256;

/// Implementation for the `PUSH0` opcode.
pub struct OpPush0Handler;
impl OpcodeHandler for OpPush0Handler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::PUSH0)?;

        vm.current_call_frame.stack.push_zero()?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `PUSHn` opcode.
pub struct OpPushHandler<const N: usize>;
impl<const N: usize> OpcodeHandler for OpPushHandler<N> {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let literal_offset = vm.current_call_frame.pc;
        vm.current_call_frame.pc = vm
            .current_call_frame
            .pc
            .checked_add(N)
            .ok_or(InternalError::Overflow)?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::PUSHN)?;

        #[expect(clippy::indexing_slicing, reason = "length is checked in match guard")]
        vm.current_call_frame.stack.push(
            match vm
                .current_call_frame
                .bytecode
                .bytecode
                .get(literal_offset..)
            {
                Some(data) if data.len() >= N => U256::from_big_endian(&data[..N]),
                Some(data) => {
                    let mut bytes = [0; 32];
                    bytes[..data.len()].copy_from_slice(data);
                    U256::from_big_endian(&bytes)
                }
                None => U256::zero(),
            },
        )?;

        Ok(OpcodeResult::Continue)
    }
}
