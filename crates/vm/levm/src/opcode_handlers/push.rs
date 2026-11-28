//! # Stack push operations
//!
//! Includes the following opcodes:
//!   - `PUSH0`
//!   - `PUSH1` to `PUSH32`

use crate::{
    constants::STACK_LIMIT,
    errors::{InternalError, OpcodeResult, VMError},
    gas_cost,
    opcode_handlers::{OpcodeHandler, stack_memory_storage_flow::jump},
    opcodes::Opcode,
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

        if N <= 2
            && let Some(result) = static_jump::<N>(vm, literal_offset, literal_end)
        {
            return result;
        }

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

/// Runs a static jump, `PUSH1`/`PUSH2 <destination>` followed by `JUMP` or `JUMPI`, as one
/// step: the shape compilers emit for every internal jump. The literal is the destination, so
/// it never goes through the stack and the `JUMP`/`JUMPI` dispatch is skipped.
///
/// Gas is charged in the same order and amounts as the two opcodes run one after the other,
/// and the resulting pc is the same. Returns `None`, leaving the pair to the unfused path,
/// when the next opcode is not a jump or when the unfused pair would fault on the stack (a
/// full stack makes the PUSH overflow; a `JUMPI` without a condition underflows), so those
/// faults stay exactly as they are. The struct-log tracer and the validation observer see
/// every opcode as its own step, so they keep the unfused path too.
#[inline(always)]
fn static_jump<const N: usize>(
    vm: &mut VM<'_>,
    literal_offset: usize,
    literal_end: usize,
) -> Option<Result<OpcodeResult, VMError>> {
    let bytecode = vm.current_call_frame.bytecode.dispatch_buf();
    // SAFETY: `literal_end` is at most `bytecode_len + N` with `N <= 2`, inside the zeroed
    // BYTECODE_PADDING (> 32 bytes) that follows every bytecode; padding reads as STOP.
    #[expect(unsafe_code, reason = "read bounded by padded bytecode len")]
    let next = unsafe { *bytecode.get_unchecked(literal_end) };
    // JUMP (0x56) and JUMPI (0x57) differ only in the lowest bit, so one compare tells a PUSH
    // that is not followed by a jump, the common case, to carry on as a plain PUSH.
    if next & !1 != u8::from(Opcode::JUMP) {
        return None;
    }
    let is_jump = next == u8::from(Opcode::JUMP);
    let stack_offset = vm.current_call_frame.stack.offset;
    if stack_offset == 0
        || (!is_jump && stack_offset == STACK_LIMIT)
        || vm.opcode_tracer.active
        || vm.validation_observer.active
    {
        return None;
    }
    let mut target = 0usize;
    for i in 0..N {
        // SAFETY: the immediates lie inside the padded bytecode, as in the PUSH read.
        #[expect(unsafe_code, reason = "read bounded by padded bytecode len")]
        let byte = unsafe { *bytecode.get_unchecked(literal_offset.wrapping_add(i)) };
        target = (target << 8) | usize::from(byte);
    }
    Some(run_static_jump(vm, is_jump, target, literal_end))
}

#[inline(always)]
fn run_static_jump(
    vm: &mut VM<'_>,
    is_jump: bool,
    target: usize,
    literal_end: usize,
) -> Result<OpcodeResult, VMError> {
    let frame = &mut vm.current_call_frame;
    frame.increase_consumed_gas(gas_cost::PUSHN)?;
    if is_jump {
        frame.increase_consumed_gas(gas_cost::JUMP)?;
        jump(vm, target, gas_cost::JUMP)?;
    } else {
        frame.increase_consumed_gas(gas_cost::JUMPI)?;
        if frame.stack.pop1()?.is_zero() {
            // Not taken: continue after the JUMPI.
            frame.pc = literal_end.wrapping_add(1);
        } else {
            jump(vm, target, gas_cost::JUMPI)?;
        }
    }
    Ok(OpcodeResult::Continue)
}
