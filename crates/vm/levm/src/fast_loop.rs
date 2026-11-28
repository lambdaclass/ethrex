//! Experiment (not for merge): an inner dispatch loop for simple opcodes.
//!
//! Each opcode the table dispatch runs stores `pc`, the remaining gas and the stack offset
//! back to the call frame and the next opcode loads them again. Those three store/load chains
//! run every opcode through ~8 cycles of memory latency even when the opcode itself is a single
//! add. [`run_simple_ops`] keeps the three in locals while it runs a straight sequence of
//! simple opcodes, and writes them back once when it stops.
//!
//! It only runs an opcode when that opcode cannot fail: enough gas, enough stack items and
//! room, a valid jump destination. Otherwise it stops *before* the opcode, leaving the frame
//! exactly as the table dispatch would find it, so the regular handler runs it and raises the
//! same error from the same state. Gas costs and results are the handlers' own.

use crate::{call_frame::CallFrame, constants::STACK_LIMIT, gas_cost};
use ethrex_common::{
    U256,
    utils::{u256_from_big_endian_const, u256_to_big_endian},
};

/// Opcodes [`run_simple_ops`] runs; anything else stops it. Only used from Shanghai on
/// (`PUSH0`), and none of these changed after that.
#[expect(
    clippy::as_conversions,
    clippy::indexing_slicing,
    reason = "const table built from in-range opcode bytes"
)]
pub(crate) static SIMPLE_OPS: [bool; 256] = {
    let mut table = [false; 256];
    let simple: &[u8] = &[
        0x01, // ADD
        0x03, // SUB
        0x10, // LT
        0x11, // GT
        0x14, // EQ
        0x15, // ISZERO
        0x16, // AND
        0x17, // OR
        0x18, // XOR
        0x19, // NOT
        0x50, // POP
        0x56, // JUMP
        0x57, // JUMPI
        0x58, // PC
        0x5a, // GAS
        0x5b, // JUMPDEST
        0x5f, // PUSH0
    ];
    let mut i = 0;
    while i < simple.len() {
        table[simple[i] as usize] = true;
        i += 1;
    }
    // PUSH1..=PUSH32, DUP1..=DUP16, SWAP1..=SWAP16.
    let mut op = 0x60;
    while op <= 0x9f {
        table[op] = true;
        op += 1;
    }
    table
};

/// [`SIMPLE_OPS`] plus MUL, MLOAD/MSTORE of words already inside the frame's memory, and the
/// frame reads ADDRESS, CALLER, CALLVALUE, CALLDATASIZE, RETURNDATASIZE and MSIZE
/// (`ETHREX_EXP_FAST_LOOP=2`).
pub(crate) static SIMPLE_OPS_MEM: [bool; 256] = {
    let mut table = SIMPLE_OPS;
    table[0x02] = true; // MUL
    table[0x51] = true; // MLOAD
    table[0x52] = true; // MSTORE
    table[0x30] = true; // ADDRESS
    table[0x33] = true; // CALLER
    table[0x34] = true; // CALLVALUE
    table[0x36] = true; // CALLDATASIZE
    table[0x3d] = true; // RETURNDATASIZE
    table[0x59] = true; // MSIZE
    table
};

/// Whether the next `run` opcodes from the frame's `pc` are all simple. Entering and leaving the
/// loop costs about as much as a few simple opcodes, so a short run between other opcodes
/// (`RETURNDATASIZE POP RETURNDATASIZE ...`, `POP PUSH0 MLOAD ...`) stays on the table dispatch.
#[inline(always)]
#[expect(unsafe_code, reason = "every read is inside the padded bytecode")]
#[expect(
    clippy::as_conversions,
    clippy::indexing_slicing,
    reason = "opcode bytes index a 256-entry table"
)]
pub(crate) fn worth_entering(frame: &CallFrame, simple_ops: &[bool; 256], run: usize) -> bool {
    let code = frame.bytecode.dispatch_buf();
    let mut pc = frame.pc;
    for _ in 0..run {
        // SAFETY: the first read is at `pc`, inside the padded bytecode (see
        // `CallFrame::next_opcode`); each later one moves past one opcode and its immediates. A
        // read can land in the zero padding only after a run of STOPs (not simple) or past a
        // PUSH, at most 33 bytes past a real byte, so it stays inside BYTECODE_PADDING (> 32).
        let op = unsafe { *code.get_unchecked(pc) };
        if !simple_ops[op as usize] {
            return false;
        }
        // The next opcode follows the immediates of a PUSH1..=PUSH32. A jump's real successor is
        // its destination, which this does not follow; the bytes after it are only a guess.
        let immediates = if (0x60..=0x7f).contains(&op) {
            usize::from(op.wrapping_sub(0x5f))
        } else {
            0
        };
        pc = pc.wrapping_add(1).wrapping_add(immediates);
    }
    true
}

/// Reads the `N` immediate bytes of a PUSH at `at` as a big-endian word.
///
/// # Safety
/// `at + N` must lie inside the padded bytecode buffer.
#[inline(always)]
#[expect(
    unsafe_code,
    reason = "caller guarantees the read is inside the padded bytecode"
)]
unsafe fn push_value<const N: usize>(code: &[u8], at: usize) -> U256 {
    let bytes: [u8; N] = unsafe { std::ptr::read_unaligned(code.as_ptr().add(at).cast()) };
    u256_from_big_endian_const(bytes)
}

#[inline(always)]
fn u256_lt(a: &U256, b: &U256) -> bool {
    let (a, b) = (&a.0, &b.0);
    (a[3], a[2], a[1], a[0]) < (b[3], b[2], b[1], b[0])
}

/// Writes a 0/1 (or any `u64`) result to a stack slot with one 32-byte store. `U256::from(u64)`
/// stores the low limb, then 16 and 8 zero bytes; the next opcode's 32-byte load of the slot
/// cannot be forwarded from three partial stores and waits for them to reach the cache.
///
/// # Safety
/// `slot` must be a valid stack slot.
#[inline(always)]
#[expect(unsafe_code, reason = "caller passes a valid stack slot")]
#[allow(
    clippy::as_conversions,
    reason = "u64 bits as the intrinsic's i64 lane (x86-64 only)"
)]
unsafe fn store_u64(slot: *mut U256, value: u64) {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx"))]
    unsafe {
        use std::arch::x86_64::{_mm256_set_epi64x, _mm256_storeu_si256};
        _mm256_storeu_si256(slot.cast(), _mm256_set_epi64x(0, 0, 0, value as i64));
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx")))]
    unsafe {
        *slot = U256::from(value);
    }
}

/// AND/OR/XOR of the words at `a` and `b`, written to `b`, as one 32-byte load per operand, one
/// vector operation and one 32-byte store (by value, `U256`'s operators move each word as four
/// limbs, so the next opcode's 32-byte load cannot be forwarded).
///
/// # Safety
/// `a` and `b` must be valid stack slots.
#[inline(always)]
#[expect(unsafe_code, reason = "caller passes valid stack slots")]
unsafe fn bitwise(a: *const U256, b: *mut U256, op: u8) {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    unsafe {
        use std::arch::x86_64::{
            _mm256_and_si256, _mm256_loadu_si256, _mm256_or_si256, _mm256_storeu_si256,
            _mm256_xor_si256,
        };
        let (x, y) = (_mm256_loadu_si256(a.cast()), _mm256_loadu_si256(b.cast()));
        let result = match op {
            0x16 => _mm256_and_si256(x, y),
            0x17 => _mm256_or_si256(x, y),
            _ => _mm256_xor_si256(x, y),
        };
        _mm256_storeu_si256(b.cast(), result);
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
    unsafe {
        *b = match op {
            0x16 => *a & *b,
            0x17 => *a | *b,
            _ => *a ^ *b,
        };
    }
}

/// Runs simple opcodes from the frame's `pc` until the next one is not simple or could fail.
#[inline(always)]
#[expect(
    unsafe_code,
    reason = "stack slots are bounds-checked against `sp` before each access; pc stays inside \
              the padded bytecode"
)]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    reason = "pc, sp and gas are checked before every change"
)]
pub(crate) fn run_simple_ops(frame: &mut CallFrame, memory_ops: bool) {
    let code = frame.bytecode.dispatch_buf();
    let values = frame.stack.values.as_mut_ptr();
    // The offset of the word at `offset` when that word is already inside the frame's memory
    // (`len` is word-rounded, so then nothing grows and only the static cost applies).
    fn in_memory(offset: &U256, memory_len: usize) -> Option<usize> {
        let offset = usize::try_from(*offset).ok()?;
        let end = offset.checked_add(32)?;
        (end.checked_next_multiple_of(32)? <= memory_len).then_some(offset)
    }
    let mut pc = frame.pc;
    let mut gas = frame.gas_remaining;
    let mut sp = frame.stack.offset;

    // Two items on the stack: slots `sp` and `sp + 1` are in bounds.
    macro_rules! has_two {
        () => {
            sp + 1 < STACK_LIMIT
        };
    }
    macro_rules! binary {
        ($cost:expr, |$a:ident, $b:ident| $result:expr) => {{
            if gas < $cost as i64 || !has_two!() {
                break;
            }
            // SAFETY: `sp + 1 < STACK_LIMIT`, so both slots are in bounds and distinct.
            let ($a, $b) = unsafe { (&*values.add(sp), &mut *values.add(sp + 1)) };
            *$b = $result;
            sp += 1;
            gas -= $cost as i64;
            pc += 1;
        }};
    }
    // A binary opcode written straight to the slots: `$a` is the popped top, `$b` the slot that
    // takes the result.
    macro_rules! binary_raw {
        ($cost:expr, |$a:ident, $b:ident| $body:expr) => {{
            if gas < $cost as i64 || !has_two!() {
                break;
            }
            // SAFETY: `sp + 1 < STACK_LIMIT`, so both slots are in bounds and distinct.
            let ($a, $b) = unsafe { (values.add(sp).cast_const(), values.add(sp + 1)) };
            $body;
            sp += 1;
            gas -= $cost as i64;
            pc += 1;
        }};
    }
    macro_rules! push {
        ($cost:expr, $value:expr) => {{
            if gas < $cost as i64 || sp == 0 {
                break;
            }
            let value = $value;
            sp -= 1;
            // SAFETY: `sp` was positive, so `sp - 1` is in bounds.
            unsafe { *values.add(sp) = value };
            gas -= $cost as i64;
        }};
    }
    macro_rules! push_n {
        ($n:literal) => {{
            // SAFETY: `pc` is a real opcode byte, so its immediates end inside the
            // BYTECODE_PADDING (> 32 zero bytes) after the code; past the end they read as
            // zero, as the PUSH handler's do.
            push!(gas_cost::PUSHN, unsafe { push_value::<$n>(code, pc + 1) });
            pc += 1 + $n;
        }};
    }

    loop {
        // SAFETY: `pc` is at most the code length plus 32 (after a PUSH32 at the last byte),
        // inside the padded buffer; padding bytes are STOP, which ends the loop.
        let op = unsafe { *code.get_unchecked(pc) };
        match op {
            0x01 => binary!(gas_cost::ADD, |a, b| a.overflowing_add(*b).0),
            0x02 if memory_ops => binary!(gas_cost::MUL, |a, b| a.overflowing_mul(*b).0),
            0x51 if memory_ops => {
                // MLOAD of a word already inside memory: no expansion, so only the static cost.
                if gas < gas_cost::MLOAD_STATIC as i64 || sp >= STACK_LIMIT {
                    break;
                }
                // SAFETY: `sp < STACK_LIMIT`.
                let top = unsafe { &mut *values.add(sp) };
                let Some(offset) = in_memory(top, frame.memory.len()) else {
                    break;
                };
                // The fast loop never resizes memory, so the base pointer is current.
                let memory = frame.memory.frame_base_ptr();
                // SAFETY: `offset + 32 <= memory.len()`, inside this frame's memory.
                let word: [u8; 32] = unsafe { std::ptr::read_unaligned(memory.add(offset).cast()) };
                *top = u256_from_big_endian_const(word);
                gas -= gas_cost::MLOAD_STATIC as i64;
                pc += 1;
            }
            0x52 if memory_ops => {
                // MSTORE of a word already inside memory (the caller turns this off in debug
                // mode, whose MSTORE handler can intercept the store).
                if gas < gas_cost::MSTORE_STATIC as i64 || !has_two!() {
                    break;
                }
                // SAFETY: `sp + 1 < STACK_LIMIT`.
                let (offset, value) = unsafe { (&*values.add(sp), &*values.add(sp + 1)) };
                let Some(offset) = in_memory(offset, frame.memory.len()) else {
                    break;
                };
                let word = u256_to_big_endian(*value);
                let memory = frame.memory.frame_base_ptr();
                // SAFETY: `offset + 32 <= memory.len()`, inside this frame's memory.
                unsafe { std::ptr::write_unaligned(memory.add(offset).cast::<[u8; 32]>(), word) };
                sp += 2;
                gas -= gas_cost::MSTORE_STATIC as i64;
                pc += 1;
            }
            0x03 => binary!(gas_cost::SUB, |a, b| a.overflowing_sub(*b).0),
            // SAFETY (the six arms below): `binary_raw!` checked both slots are in bounds.
            0x10 => binary_raw!(gas_cost::LT, |a, b| unsafe {
                store_u64(b, u256_lt(&*a, &*b) as u64)
            }),
            0x11 => binary_raw!(gas_cost::GT, |a, b| unsafe {
                store_u64(b, u256_lt(&*b, &*a) as u64)
            }),
            0x14 => binary_raw!(gas_cost::EQ, |a, b| unsafe {
                store_u64(b, ((*a).0 == (*b).0) as u64)
            }),
            0x16 => binary_raw!(gas_cost::AND, |a, b| unsafe { bitwise(a, b, 0x16) }),
            0x17 => binary_raw!(gas_cost::OR, |a, b| unsafe { bitwise(a, b, 0x17) }),
            0x18 => binary_raw!(gas_cost::XOR, |a, b| unsafe { bitwise(a, b, 0x18) }),
            0x15 | 0x19 => {
                let cost = if op == 0x15 {
                    gas_cost::ISZERO
                } else {
                    gas_cost::NOT
                };
                if gas < cost as i64 || sp >= STACK_LIMIT {
                    break;
                }
                // SAFETY: `sp < STACK_LIMIT`.
                let top = unsafe { &mut *values.add(sp) };
                *top = if op == 0x15 {
                    U256::from(top.is_zero() as u64)
                } else {
                    !*top
                };
                gas -= cost as i64;
                pc += 1;
            }
            0x50 => {
                if gas < gas_cost::POP as i64 || sp >= STACK_LIMIT {
                    break;
                }
                sp += 1;
                gas -= gas_cost::POP as i64;
                pc += 1;
            }
            0x56 => {
                // JUMP, with the destination's JUMPDEST charged and skipped as `jump` does.
                let cost = gas_cost::JUMP + gas_cost::JUMPDEST;
                if gas < cost as i64 || sp >= STACK_LIMIT {
                    break;
                }
                // SAFETY: `sp < STACK_LIMIT`.
                let target = unsafe { &*values.add(sp) };
                let Ok(target) = usize::try_from(*target) else {
                    break;
                };
                if !frame.bytecode.is_valid_jumpdest(target) {
                    break;
                }
                sp += 1;
                gas -= cost as i64;
                pc = target + 1;
            }
            0x57 => {
                // JUMPI; a taken jump also charges and skips the destination's JUMPDEST.
                let cost = gas_cost::JUMPI + gas_cost::JUMPDEST;
                if gas < cost as i64 || !has_two!() {
                    break;
                }
                // SAFETY: `sp + 1 < STACK_LIMIT`.
                let (target, condition) = unsafe { (&*values.add(sp), &*values.add(sp + 1)) };
                if condition.is_zero() {
                    gas -= gas_cost::JUMPI as i64;
                    pc += 1;
                } else {
                    let Ok(target) = usize::try_from(*target) else {
                        break;
                    };
                    if !frame.bytecode.is_valid_jumpdest(target) {
                        break;
                    }
                    gas -= cost as i64;
                    pc = target + 1;
                }
                sp += 2;
            }
            0x58 => {
                push!(gas_cost::PC, U256::from(pc as u64));
                pc += 1;
            }
            0x5a => {
                // GAS pushes what is left after its own charge.
                push!(
                    gas_cost::GAS,
                    U256::from((gas - gas_cost::GAS as i64).cast_unsigned())
                );
                pc += 1;
            }
            0x30 if memory_ops => {
                push!(
                    gas_cost::ADDRESS,
                    U256::from_big_endian(frame.to.as_bytes())
                );
                pc += 1;
            }
            0x33 if memory_ops => {
                push!(
                    gas_cost::CALLER,
                    U256::from_big_endian(frame.msg_sender.as_bytes())
                );
                pc += 1;
            }
            0x34 if memory_ops => {
                push!(gas_cost::CALLVALUE, frame.msg_value);
                pc += 1;
            }
            0x36 if memory_ops => {
                push!(gas_cost::CALLDATASIZE, U256::from(frame.calldata.len()));
                pc += 1;
            }
            0x3d if memory_ops => {
                push!(
                    gas_cost::RETURNDATASIZE,
                    U256::from(frame.sub_return_data.len())
                );
                pc += 1;
            }
            0x59 if memory_ops => {
                push!(gas_cost::MSIZE, U256::from(frame.memory.len()));
                pc += 1;
            }
            0x5b => {
                if gas < gas_cost::JUMPDEST as i64 {
                    break;
                }
                gas -= gas_cost::JUMPDEST as i64;
                pc += 1;
            }
            0x5f => {
                push!(gas_cost::PUSH0, U256::zero());
                pc += 1;
            }
            0x60 => push_n!(1),
            0x61 => push_n!(2),
            0x62 => push_n!(3),
            0x63 => push_n!(4),
            0x64 => push_n!(5),
            0x65 => push_n!(6),
            0x66 => push_n!(7),
            0x67 => push_n!(8),
            0x68 => push_n!(9),
            0x69 => push_n!(10),
            0x6a => push_n!(11),
            0x6b => push_n!(12),
            0x6c => push_n!(13),
            0x6d => push_n!(14),
            0x6e => push_n!(15),
            0x6f => push_n!(16),
            0x70 => push_n!(17),
            0x71 => push_n!(18),
            0x72 => push_n!(19),
            0x73 => push_n!(20),
            0x74 => push_n!(21),
            0x75 => push_n!(22),
            0x76 => push_n!(23),
            0x77 => push_n!(24),
            0x78 => push_n!(25),
            0x79 => push_n!(26),
            0x7a => push_n!(27),
            0x7b => push_n!(28),
            0x7c => push_n!(29),
            0x7d => push_n!(30),
            0x7e => push_n!(31),
            0x7f => push_n!(32),
            0x80..=0x8f => {
                // DUPn copies the item at depth n - 1.
                let depth = usize::from(op - 0x80);
                if gas < gas_cost::DUPN as i64 || sp == 0 || sp + depth >= STACK_LIMIT {
                    break;
                }
                // SAFETY: `sp + depth < STACK_LIMIT` and `sp - 1 >= 0`, distinct slots.
                unsafe {
                    std::ptr::copy_nonoverlapping(values.add(sp + depth), values.add(sp - 1), 1)
                };
                sp -= 1;
                gas -= gas_cost::DUPN as i64;
                pc += 1;
            }
            0x90..=0x9f => {
                // SWAPn exchanges the top with the item at depth n.
                let depth = usize::from(op - 0x8f);
                if gas < gas_cost::SWAPN as i64 || sp + depth >= STACK_LIMIT {
                    break;
                }
                // SAFETY: `sp < sp + depth < STACK_LIMIT`: distinct, in-bounds slots. Two
                // vector loads and stores, as `Stack::swap_distinct_unchecked` does, keep both
                // values in registers instead of a round trip through a stack temporary.
                unsafe {
                    let (a, b) = (values.add(sp), values.add(sp + depth));
                    #[cfg(all(target_arch = "x86_64", target_feature = "avx"))]
                    {
                        use std::arch::x86_64::{__m256i, _mm256_loadu_si256, _mm256_storeu_si256};
                        let (a, b) = (a.cast::<__m256i>(), b.cast::<__m256i>());
                        let (a_value, b_value) = (_mm256_loadu_si256(a), _mm256_loadu_si256(b));
                        _mm256_storeu_si256(a, b_value);
                        _mm256_storeu_si256(b, a_value);
                    }
                    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx")))]
                    std::ptr::swap_nonoverlapping(a, b, 1);
                }
                gas -= gas_cost::SWAPN as i64;
                pc += 1;
            }
            _ => break,
        }
    }

    frame.pc = pc;
    frame.gas_remaining = gas;
    frame.stack.offset = sp;
}
