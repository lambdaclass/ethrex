//! # Stack push operations
//!
//! Includes the following opcodes:
//!   - `PUSH0`
//!   - `PUSH1` to `PUSH32`

use crate::{
    errors::{OpcodeResult, VMError},
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

/// Specialized handler for `PUSH1`, the most common push opcode.
///
/// Avoids the overhead of `U256::from_big_endian` by directly constructing
/// a U256 from a single byte. Safe because bytecode has 33 bytes of zero padding.
pub struct OpPush1Handler;
impl OpcodeHandler for OpPush1Handler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let pc = vm.current_call_frame.pc;
        vm.current_call_frame.pc = pc.wrapping_add(1);

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::PUSHN)?;

        #[expect(
            unsafe_code,
            reason = "bytecode is padded with 33 zero bytes past code_len"
        )]
        let byte = unsafe { *vm.current_call_frame.bytecode.bytecode.get_unchecked(pc) };

        vm.current_call_frame
            .stack
            .push(U256::from(u64::from(byte)))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Specialized handler for `PUSH2`, a very common push opcode.
///
/// Avoids the overhead of generic `U256::from_big_endian` by constructing
/// a U256 directly from two bytes. Safe because bytecode has 33 bytes of zero padding.
pub struct OpPush2Handler;
impl OpcodeHandler for OpPush2Handler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let pc = vm.current_call_frame.pc;
        vm.current_call_frame.pc = pc.wrapping_add(2);

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::PUSHN)?;

        #[expect(
            unsafe_code,
            reason = "bytecode is padded with 33 zero bytes past code_len"
        )]
        let (b0, b1) = unsafe {
            let bytecode = &vm.current_call_frame.bytecode.bytecode;
            (
                *bytecode.get_unchecked(pc),
                *bytecode.get_unchecked(pc.wrapping_add(1)),
            )
        };
        let value = (u64::from(b0) << 8) | u64::from(b1);

        vm.current_call_frame.stack.push(U256::from(value))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `PUSHn` opcode (PUSH3 through PUSH32).
///
/// Safe to use unchecked indexing because bytecode is padded with 33 zero bytes,
/// which is >= the maximum N (32).
pub struct OpPushHandler<const N: usize>;
impl<const N: usize> OpcodeHandler for OpPushHandler<N> {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let pc = vm.current_call_frame.pc;
        vm.current_call_frame.pc = pc.wrapping_add(N);

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::PUSHN)?;

        #[expect(
            unsafe_code,
            reason = "bytecode is padded with 33 zero bytes past code_len, N <= 32"
        )]
        let data = unsafe {
            core::slice::from_raw_parts(vm.current_call_frame.bytecode.bytecode.as_ptr().add(pc), N)
        };
        vm.current_call_frame
            .stack
            .push(U256::from_big_endian(data))?;

        Ok(OpcodeResult::Continue)
    }
}
