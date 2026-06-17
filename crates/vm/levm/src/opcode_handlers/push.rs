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
use ethrex_common::{types::BYTECODE_PADDING, utils::u256_from_big_endian_const};

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
        // PUSHn reads up to 32 immediate bytes without a bounds check, relying on
        // the trailing zero padding appended to every bytecode. After the read the
        // dispatch loop fetches the next opcode at `pc + N`, which needs one byte
        // beyond the immediates, so the padding must exceed 32 (i.e. be >= 33).
        // Keep the unchecked read below sound if the padding ever shrinks.
        const { assert!(BYTECODE_PADDING > 32) };

        let literal_offset = vm.current_call_frame.pc;
        // Use a *checked* add for the pc advance, not unchecked `+= N`. Both
        // compute the same value (pc never overflows in practice), but the
        // checked form is required for good codegen here: with unchecked/wrapping
        // arithmetic LLVM can no longer prove the immediate slice length is the
        // constant `N`, so the `get_unchecked` read below degrades to a
        // runtime-length memcpy and the PUSH hot loop runs ~2x slower (IPC
        // collapses 3.4 -> 1.2). The overflow branch is free (perfectly
        // predicted) and never taken.
        vm.current_call_frame.pc = literal_offset
            .checked_add(N)
            .ok_or(InternalError::Overflow)?;
        // `pc` is now exactly `literal_offset + N`; reuse it as the immediate end.
        let literal_end = vm.current_call_frame.pc;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::PUSHN)?;

        let bytecode = vm.current_call_frame.bytecode.dispatch_buf();
        // SAFETY: PUSH only dispatches on a real opcode byte, so
        // `literal_offset <= bytecode_len`; the buffer is padded with
        // BYTECODE_PADDING (>= N) trailing zeros, so N bytes are always in bounds.
        // Immediate bytes past the real code end read as zero, matching EVM
        // PUSH-past-end semantics (the padding is zeroed).
        let mut buf = [0u8; N];
        #[expect(unsafe_code, reason = "read bounded by padded bytecode len")]
        buf.copy_from_slice(unsafe { bytecode.get_unchecked(literal_offset..literal_end) });
        let value = u256_from_big_endian_const(buf);
        vm.current_call_frame.stack.push(value)?;

        Ok(OpcodeResult::Continue)
    }
}
