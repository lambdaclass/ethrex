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
use ethrex_common::{U256, utils::u256_from_big_endian_const};

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

        let bytecode = &vm.current_call_frame.bytecode.bytecode;
        let value = match bytecode.get(literal_offset..) {
            #[expect(clippy::indexing_slicing, reason = "length is checked in match guard")]
            Some(data) if data.len() >= N => {
                let mut buf = [0u8; N];
                buf.copy_from_slice(&data[..N]);
                u256_from_big_endian_const(buf)
            }
            Some(data) => {
                let mut bytes = [0u8; N];
                bytes[..data.len()].copy_from_slice(data);
                u256_from_big_endian_const(bytes)
            }
            None => U256::zero(),
        };
        vm.current_call_frame.stack.push(value)?;

        Ok(OpcodeResult::Continue)
    }
}
