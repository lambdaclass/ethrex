//! # Arithmetic operations
//!
//! Includes the following opcodes:
//!   - `ADD`
//!   - `SUB`
//!   - `MUL`
//!   - `DIV`
//!   - `SDIV`
//!   - `MOD`
//!   - `SMOD`
//!   - `ADDMOD`
//!   - `MULMOD`
//!   - `EXP`
//!   - `SIGNEXTEND`
//!   - `CLZ`

use crate::{
    errors::{OpcodeResult, VMError},
    gas_cost,
    opcode_handlers::OpcodeHandler,
    vm::VM,
};
use ethrex_common::U256;
use ethrex_crypto::u256_div;

/// Implementation for the `ADD` opcode.
pub struct OpAddHandler;
impl OpcodeHandler for OpAddHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::ADD)?;

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        *rhs = lhs.overflowing_add(*rhs).0;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SUB` opcode.
pub struct OpSubHandler;
impl OpcodeHandler for OpSubHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::SUB)?;

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        *rhs = lhs.overflowing_sub(*rhs).0;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `MUL` opcode.
pub struct OpMulHandler;
impl OpcodeHandler for OpMulHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::MUL)?;

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        *rhs = wrapping_mul(&lhs, rhs);

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `DIV` opcode.
pub struct OpDivHandler;
impl OpcodeHandler for OpDivHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::DIV)?;

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        *rhs = u256_div::div(lhs, *rhs);

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SDIV` opcode.
pub struct OpSDivHandler;
impl OpcodeHandler for OpSDivHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::SDIV)?;

        let (top, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        let mut lhs = top;
        let mut rhs = *slot;

        let mut sign = false;
        if lhs.bit(255) {
            lhs = U256::zero().overflowing_sub(lhs).0;
            sign = !sign;
        }
        if rhs.bit(255) {
            rhs = U256::zero().overflowing_sub(rhs).0;
            sign = !sign;
        }

        let mut res = u256_div::div(lhs, rhs);
        if sign {
            res = U256::zero().overflowing_sub(res).0;
        }
        *slot = res;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `MOD` opcode.
pub struct OpModHandler;
impl OpcodeHandler for OpModHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::MOD)?;

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        *rhs = u256_div::rem(lhs, *rhs);

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SMOD` opcode.
pub struct OpSModHandler;
impl OpcodeHandler for OpSModHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::SMOD)?;

        let (top, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        let mut lhs = top;
        let mut rhs = *slot;

        let sign = lhs.bit(255);
        if sign {
            (lhs, _) = (!lhs).overflowing_add(U256::one());
        }
        if rhs.bit(255) {
            (rhs, _) = (!rhs).overflowing_add(U256::one());
        }

        let mut res = u256_div::rem(lhs, rhs);
        if sign {
            (res, _) = (!res).overflowing_add(U256::one());
        }
        *slot = res;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `ADDMOD` opcode.
pub struct OpAddModHandler;
impl OpcodeHandler for OpAddModHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::ADDMOD)?;

        let [lhs, rhs, r#mod] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame
            .stack
            .push(u256_div::add_mod(lhs, rhs, r#mod))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `MULMOD` opcode.
pub struct OpMulModHandler;
impl OpcodeHandler for OpMulModHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::MULMOD)?;

        let [multiplicand, multiplier, modulus] = *vm.current_call_frame.stack.pop()?;
        if modulus.is_zero() || multiplicand.is_zero() || multiplier.is_zero() {
            vm.current_call_frame.stack.push_zero()?;
        } else {
            let a_bytes = multiplicand.to_big_endian();
            let b_bytes = multiplier.to_big_endian();
            let m_bytes = modulus.to_big_endian();
            let result_bytes = vm.crypto.mulmod256(&a_bytes, &b_bytes, &m_bytes);
            let product_mod = U256::from_big_endian(&result_bytes);
            vm.current_call_frame.stack.push(product_mod)?;
        }

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `EXP` opcode.
pub struct OpExpHandler;
impl OpcodeHandler for OpExpHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [base, exp] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::exp(exp)?)?;

        vm.current_call_frame.stack.push(wrapping_pow(base, exp))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SIGNEXTEND` opcode.
pub struct OpSignExtendHandler;
impl OpcodeHandler for OpSignExtendHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::SIGNEXTEND)?;

        let (index, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        *slot = sign_extend(index, *slot);

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `CLZ` opcode.
pub struct OpClzHandler;
impl OpcodeHandler for OpClzHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::CLZ)?;

        let slot = vm.current_call_frame.stack.top_mut()?;
        let lz = slot.leading_zeros();
        *slot = lz.into();

        Ok(OpcodeResult::Continue)
    }
}

/// Low 256 bits of `a * b`. Only the ten partial products below 2^256 are computed; the
/// full 512-bit product needs sixteen.
#[inline(always)]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::indexing_slicing,
    reason = "i + j < 4, and a 64x64-bit product plus two words fits in 128 bits"
)]
fn wrapping_mul(a: &U256, b: &U256) -> U256 {
    let mut product = [0u64; 4];
    for i in 0..4 {
        let mut carry = 0u64;
        for j in 0..4 - i {
            let t = u128::from(a.0[i]) * u128::from(b.0[j])
                + u128::from(product[i + j])
                + u128::from(carry);
            product[i + j] = t as u64;
            carry = (t >> 64) as u64;
        }
    }
    U256(product)
}

/// `base^exponent mod 2^256`.
///
/// Powers with a closed form skip square-and-multiply, which otherwise costs up to 255
/// squarings for a full-width exponent: bases 0, 1 and 2^256 - 1, powers of two, and even
/// bases raised far enough to wrap to zero.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "shift amounts and the exponent bit index stay below 256"
)]
fn wrapping_pow(base: U256, exponent: U256) -> U256 {
    if exponent.is_zero() {
        return U256::one();
    }
    if base <= U256::one() || exponent == U256::one() {
        return base;
    }
    // (2^256 - 1)^e is (-1)^e.
    if base == U256::MAX {
        return if exponent.bit(0) {
            U256::MAX
        } else {
            U256::one()
        };
    }
    let trailing_zeros = base.trailing_zeros();
    if trailing_zeros > 0 {
        // base = 2^tz * odd, so base^e is a multiple of 2^(tz * e): zero once that
        // reaches 2^256.
        let shift = match u32::try_from(exponent) {
            Ok(e) if e < 256 => trailing_zeros * e,
            _ => return U256::zero(),
        };
        if shift >= 256 {
            return U256::zero();
        }
        if base == U256::one() << trailing_zeros {
            return U256::one() << shift;
        }
    }

    // Left-to-right square-and-multiply over the exponent bits below the top one.
    let mut result = base;
    for i in (0..exponent.bits() - 1).rev() {
        result = wrapping_mul(&result, &result);
        if exponent.bit(i) {
            result = wrapping_mul(&result, &base);
        }
    }
    result
}

/// Extends the sign bit of byte `byte_index` (counted from the least significant byte)
/// through the upper bytes of `value`. Indexes past 30 leave `value` unchanged.
///
/// Each limb is selected with constant indexes, so the result stays in registers instead
/// of being patched in memory and read back.
#[inline(always)]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "byte_index < 31, so the sign bit is below 255 and its word below 4"
)]
fn sign_extend(byte_index: U256, value: U256) -> U256 {
    let byte_index = match usize::try_from(byte_index) {
        Ok(i) if i < 31 => i,
        _ => return value,
    };
    let sign_bit = byte_index * 8 + 7;
    let word = sign_bit / 64;
    let bit = sign_bit % 64;
    // All ones when the sign bit is set.
    let fill = 0u64.wrapping_sub((value.0[word] >> bit) & 1);
    // Bits up to and including the sign bit within its word.
    let kept = u64::MAX >> (63 - bit);
    let limb = |i: usize| match i.cmp(&word) {
        core::cmp::Ordering::Less => value.0[i],
        core::cmp::Ordering::Equal => (value.0[i] & kept) | (fill & !kept),
        core::cmp::Ordering::Greater => fill,
    };
    U256([limb(0), limb(1), limb(2), limb(3)])
}

#[cfg(test)]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    reason = "test operand generation"
)]
mod tests {
    use super::*;

    /// SplitMix64: deterministic operands without a dev-dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        fn word(&mut self) -> U256 {
            let bits = (self.next() % 257) as usize;
            let x = U256([self.next(), self.next(), self.next(), self.next()]);
            if bits < 256 {
                x & ((U256::one() << bits) - 1)
            } else {
                x
            }
        }
    }

    #[test]
    fn wrapping_mul_matches_full_product() {
        let mut rng = Rng(1);
        for _ in 0..10_000 {
            let (a, b) = (rng.word(), rng.word());
            assert_eq!(
                wrapping_mul(&a, &b),
                a.overflowing_mul(b).0,
                "{a:#x} * {b:#x}"
            );
        }
        assert_eq!(wrapping_mul(&U256::MAX, &U256::MAX), U256::one());
    }

    #[test]
    fn wrapping_pow_matches_square_and_multiply() {
        let max = U256::MAX;
        let bases = [
            U256::zero(),
            U256::one(),
            U256::from(2),
            U256::from(3),
            U256::from(6),
            U256::one() << 255,
            U256::from(3) << 254,
            max,
            max - 1,
            max - 2,
        ];
        let exponents = [
            U256::zero(),
            U256::one(),
            U256::from(2),
            U256::from(3),
            U256::from(127),
            U256::from(128),
            U256::from(255),
            U256::from(256),
            U256::from(u32::MAX),
            U256::one() << 32,
            max - 1,
            max,
        ];
        for base in bases {
            for exponent in exponents {
                assert_eq!(
                    wrapping_pow(base, exponent),
                    base.overflowing_pow(exponent).0,
                    "{base:#x} ^ {exponent:#x}"
                );
            }
        }
        let mut rng = Rng(2);
        for _ in 0..500 {
            let (mut base, exponent) = (rng.word(), rng.word());
            match rng.next() % 3 {
                0 => base = U256::one() << (rng.next() % 256),
                1 => base <<= rng.next() % 8,
                _ => {}
            }
            assert_eq!(
                wrapping_pow(base, exponent),
                base.overflowing_pow(exponent).0,
                "{base:#x} ^ {exponent:#x}"
            );
        }
    }

    #[test]
    fn sign_extend_matches_definition() {
        let mut rng = Rng(3);
        for _ in 0..2_000 {
            let value = rng.word();
            for index in (0..34).chain([u64::MAX]) {
                // Reference: keep the low (index + 1) bytes, fill the rest with the sign bit.
                let expected = if index < 31 {
                    let bits = 8 * (index as usize + 1);
                    let low = value & ((U256::one() << bits) - 1);
                    if value.bit(bits - 1) {
                        low | (U256::MAX << bits)
                    } else {
                        low
                    }
                } else {
                    value
                };
                assert_eq!(
                    sign_extend(U256::from(index), value),
                    expected,
                    "signextend({index}, {value:#x})"
                );
            }
        }
        assert_eq!(
            sign_extend(U256::one() << 64, U256::from(0x80)),
            U256::from(0x80)
        );
        assert_eq!(
            sign_extend(U256::from(3), U256::from(0xffda_dadau64)),
            U256::MAX - 0x25_2525
        );
    }
}
