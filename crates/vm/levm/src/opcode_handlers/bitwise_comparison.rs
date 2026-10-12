//! # Bitwise and comparison operations
//!
//! Includes the following opcodes:
//!   - `LT`
//!   - `GT`
//!   - `SLT`
//!   - `SGT`
//!   - `EQ`
//!   - `ISZERO`
//!   - `AND`
//!   - `OR`
//!   - `XOR`
//!   - `NOT`
//!   - `BYTE`
//!   - `SHL`
//!   - `SHR`
//!   - `SAR`

use crate::{
    errors::{OpcodeResult, VMError},
    gas_cost,
    opcode_handlers::OpcodeHandler,
    vm::VM,
};
use ethrex_common::U256;

/// Implementation for the `LT` opcode.
pub struct OpLtHandler;
impl OpcodeHandler for OpLtHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::LT)?;

        let (&lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        #[expect(clippy::as_conversions, reason = "safe")]
        let res = (lhs < *rhs) as u64;
        store_u64(rhs, res);

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `GT` opcode.
pub struct OpGtHandler;
impl OpcodeHandler for OpGtHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::GT)?;

        let (&lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        #[expect(clippy::as_conversions, reason = "safe")]
        let res = (lhs > *rhs) as u64;
        store_u64(rhs, res);

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SLT` opcode.
pub struct OpSLtHandler;
impl OpcodeHandler for OpSLtHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::SLT)?;

        let (&lhs, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        let rhs = *slot;
        let lhs_sign = lhs.bit(255);
        let rhs_sign = rhs.bit(255);

        let res: u64 = match (lhs_sign, rhs_sign) {
            (false, true) => 0,
            (true, false) => 1,
            #[expect(clippy::as_conversions, reason = "safe")]
            _ => (lhs < rhs) as u64,
        };
        store_u64(slot, res);

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SGT` opcode.
pub struct OpSGtHandler;
impl OpcodeHandler for OpSGtHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::SGT)?;

        let (&lhs, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        let rhs = *slot;
        let lhs_sign = lhs.bit(255);
        let rhs_sign = rhs.bit(255);

        let res: u64 = match (lhs_sign, rhs_sign) {
            (false, true) => 1,
            (true, false) => 0,
            #[expect(clippy::as_conversions, reason = "safe")]
            _ => (lhs > rhs) as u64,
        };
        store_u64(slot, res);

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `EQ` opcode.
pub struct OpEqHandler;
impl OpcodeHandler for OpEqHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::EQ)?;

        let (&lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        #[expect(clippy::as_conversions, reason = "safe")]
        let res = (lhs == *rhs) as u64;
        store_u64(rhs, res);

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `ISZERO` opcode.
pub struct OpIsZeroHandler;
impl OpcodeHandler for OpIsZeroHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::ISZERO)?;

        // In-place top mutation: no pop/push, no `offset` write.
        let slot = vm.current_call_frame.stack.top_mut()?;
        #[expect(clippy::as_conversions, reason = "safe")]
        let z = slot.is_zero() as u64;
        *slot = z.into();

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `AND` opcode.
pub struct OpAndHandler;
impl OpcodeHandler for OpAndHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::AND)?;

        let (&lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        *rhs = lhs & *rhs;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `OR` opcode.
pub struct OpOrHandler;
impl OpcodeHandler for OpOrHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::OR)?;

        let (&lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        *rhs = lhs | *rhs;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `XOR` opcode.
pub struct OpXorHandler;
impl OpcodeHandler for OpXorHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::XOR)?;

        let (&lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        *rhs = lhs ^ *rhs;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `NOT` opcode.
pub struct OpNotHandler;
impl OpcodeHandler for OpNotHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::NOT)?;

        // In-place top mutation: no pop/push, no `offset` write.
        let slot = vm.current_call_frame.stack.top_mut()?;
        *slot = !*slot;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `BYTE` opcode.
pub struct OpByteHandler;
impl OpcodeHandler for OpByteHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::BYTE)?;

        let (&index, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        *slot = match u8::try_from(index) {
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "x < 32 guard prevents overflow"
            )]
            Ok(x) if x < 32 => U256::from(shift_right(slot, 8 * (31 - x), 0).0[0] & 0xff),
            _ => U256::zero(),
        };

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SHL` opcode.
pub struct OpShlHandler;
impl OpcodeHandler for OpShlHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::SHL)?;

        let (&shift_amount, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        *slot = match u8::try_from(shift_amount) {
            Ok(shift_amount) => shift_left(slot, shift_amount),
            Err(_) => U256::zero(),
        };

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SHR` opcode.
pub struct OpShrHandler;
impl OpcodeHandler for OpShrHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::SHR)?;

        let (&shift_amount, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        *slot = match u8::try_from(shift_amount) {
            Ok(shift_amount) => shift_right(slot, shift_amount, 0),
            Err(_) => U256::zero(),
        };

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SAR` opcode.
pub struct OpSarHandler;
impl OpcodeHandler for OpSarHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::SAR)?;

        let (&shift_amount, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        // All ones when negative: the bits shifted in from above.
        let fill = 0u64.wrapping_sub(slot.0[3] >> 63);
        *slot = match u8::try_from(shift_amount) {
            Ok(shift_amount) => shift_right(slot, shift_amount, fill),
            Err(_) => U256([fill; 4]),
        };

        Ok(OpcodeResult::Continue)
    }
}

/// Writes a comparison result (0 or 1) to a stack slot with a single 32-byte store.
///
/// `U256::from(u64)` stores the low limb and the zeroed upper limbs separately. The next
/// opcode reads the slot with one 32-byte load, which cannot be forwarded from several
/// narrower stores and waits for them to commit.
#[inline(always)]
fn store_u64(slot: &mut U256, value: u64) {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx"))]
    #[expect(
        unsafe_code,
        reason = "a U256 is 32 bytes, so one unaligned 32-byte store covers exactly the slot"
    )]
    unsafe {
        use std::arch::x86_64::{_mm256_set_epi64x, _mm256_storeu_si256};
        let value = i64::from_ne_bytes(value.to_ne_bytes());
        _mm256_storeu_si256(
            std::ptr::from_mut(slot).cast(),
            _mm256_set_epi64x(0, 0, 0, value),
        );
    }
    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx")))]
    {
        *slot = U256::from(value);
    }
}

/// `value >> shift`, filling the vacated high bits with `fill` (zero for SHR and BYTE,
/// the sign for SAR).
///
/// Each word offset gets its own branch, so every limb index is a constant and the value
/// stays in registers: a limb index computed at run time sends it through a stack
/// temporary, which the next load has to wait on.
#[inline(always)]
fn shift_right(value: &U256, shift: u8, fill: u64) -> U256 {
    let bits = u32::from(shift % 64);
    match shift / 64 {
        0 => shift_right_words::<0>(&value.0, bits, fill),
        1 => shift_right_words::<1>(&value.0, bits, fill),
        2 => shift_right_words::<2>(&value.0, bits, fill),
        _ => shift_right_words::<3>(&value.0, bits, fill),
    }
}

/// `value << shift`, see [`shift_right`].
#[inline(always)]
fn shift_left(value: &U256, shift: u8) -> U256 {
    let bits = u32::from(shift % 64);
    match shift / 64 {
        0 => shift_left_words::<0>(&value.0, bits),
        1 => shift_left_words::<1>(&value.0, bits),
        2 => shift_left_words::<2>(&value.0, bits),
        _ => shift_left_words::<3>(&value.0, bits),
    }
}

/// Shifts right by `WORDS` limbs plus `bits < 64` bits.
#[inline(always)]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "limb indexes and bits < 64 are bounded"
)]
fn shift_right_words<const WORDS: usize>(limbs: &[u64; 4], bits: u32, fill: u64) -> U256 {
    let word = |i: usize| limbs.get(i).copied().unwrap_or(fill);
    let mut out = [fill; 4];
    for (i, limb) in out.iter_mut().enumerate().take(4 - WORDS) {
        *limb = funnel_right(word(i + WORDS + 1), word(i + WORDS), bits);
    }
    U256(out)
}

/// Shifts left by `WORDS` limbs plus `bits < 64` bits.
#[inline(always)]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "limb indexes and bits < 64 are bounded"
)]
fn shift_left_words<const WORDS: usize>(limbs: &[u64; 4], bits: u32) -> U256 {
    let word = |i: usize| limbs.get(i).copied().unwrap_or(0);
    let mut out = [0u64; 4];
    for (i, limb) in out.iter_mut().enumerate().skip(WORDS) {
        let low = if i > WORDS { word(i - WORDS - 1) } else { 0 };
        *limb = (word(i - WORDS) << bits) | ((low >> 1) >> (63 - bits));
    }
    U256(out)
}

/// The low word of `(high:low) >> bits` for `bits < 64`, without branching on `bits == 0`.
#[inline(always)]
#[expect(clippy::arithmetic_side_effects, reason = "bits < 64")]
fn funnel_right(high: u64, low: u64, bits: u32) -> u64 {
    (low >> bits) | ((high << 1) << (63 - bits))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(
        clippy::arithmetic_side_effects,
        reason = "test values: shifts and a wrapping LCG on U256"
    )]
    fn values() -> Vec<U256> {
        let mut values = vec![
            U256::zero(),
            U256::one(),
            U256::MAX,
            U256::one() << 255,
            U256::from_big_endian(&[0x80; 32]),
            U256::from_big_endian(&[0x7f; 32]),
        ];
        let mut x = U256::from(0x9e37_79b9_7f4a_7c15u64);
        for _ in 0..16 {
            x = x.overflowing_mul(U256::from(0x5851_f42d_4c95_7f2du64)).0
                ^ (x >> 7)
                ^ U256::from(0x1405_7b7e_f767_814fu64);
            values.push(x);
        }
        values
    }

    #[test]
    fn shifts_match_u256() {
        for value in values() {
            let negative = value.bit(255);
            for shift in 0u8..=255 {
                assert_eq!(
                    shift_left(&value, shift),
                    value << shift,
                    "{value:x} << {shift}"
                );
                assert_eq!(
                    shift_right(&value, shift, 0),
                    value >> shift,
                    "{value:x} >> {shift}"
                );
                let sar = if negative {
                    !(!value >> shift)
                } else {
                    value >> shift
                };
                let fill = if negative { u64::MAX } else { 0 };
                assert_eq!(
                    shift_right(&value, shift, fill),
                    sar,
                    "{value:x} sar {shift}"
                );
            }
        }
    }

    #[test]
    fn byte_extraction_matches_u256() {
        for value in values() {
            for x in 0u8..32 {
                let expected = U256::from(value.byte(usize::from(31 - x)));
                let got = U256::from(shift_right(&value, 8 * (31 - x), 0).0[0] & 0xff);
                assert_eq!(got, expected, "{value:x} byte {x}");
            }
        }
    }
}
