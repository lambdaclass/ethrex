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

/// Inline limb-wise U256 `<` (limb[3] most significant). The derived `PartialOrd`
/// on `U256`/`[u64;4]` lowers to an out-of-line `partial_cmp`/`memcmp` **call**
/// (a constrained AIR call body) that also forces spilling `lhs` to an address;
/// this frameless short-circuit form deletes both.
#[inline(always)]
fn u256_lt(a: &[u64; 4], b: &[u64; 4]) -> bool {
    if a[3] != b[3] {
        return a[3] < b[3];
    }
    if a[2] != b[2] {
        return a[2] < b[2];
    }
    if a[1] != b[1] {
        return a[1] < b[1];
    }
    a[0] < b[0]
}

/// Inline limb-wise U256 `==` (branchless xor-or), avoiding the derived
/// `PartialEq`'s out-of-line `memcmp` call.
#[inline(always)]
fn u256_eq(a: &[u64; 4], b: &[u64; 4]) -> bool {
    ((a[0] ^ b[0]) | (a[1] ^ b[1]) | (a[2] ^ b[2]) | (a[3] ^ b[3])) == 0
}

/// Implementation for the `LT` opcode.
pub struct OpLtHandler;
impl OpcodeHandler for OpLtHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::LT)?;

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut_scalars()?;
        #[expect(clippy::as_conversions, reason = "safe")]
        let res = u256_lt(&lhs.0, &rhs.0) as u64;
        *rhs = res.into();

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `GT` opcode.
pub struct OpGtHandler;
impl OpcodeHandler for OpGtHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::GT)?;

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut_scalars()?;
        #[expect(clippy::as_conversions, reason = "safe")]
        let res = u256_lt(&rhs.0, &lhs.0) as u64;
        *rhs = res.into();

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SLT` opcode.
pub struct OpSLtHandler;
impl OpcodeHandler for OpSLtHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::SLT)?;

        let (lhs, slot) = vm.current_call_frame.stack.pop1_and_top_mut_scalars()?;
        let rhs = *slot;
        let lhs_sign = lhs.bit(255);
        let rhs_sign = rhs.bit(255);

        *slot = match (lhs_sign, rhs_sign) {
            (false, true) => U256::zero(),
            (true, false) => U256::one(),
            #[expect(clippy::as_conversions, reason = "safe")]
            _ => (u256_lt(&lhs.0, &rhs.0) as u64).into(),
        };

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SGT` opcode.
pub struct OpSGtHandler;
impl OpcodeHandler for OpSGtHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::SGT)?;

        let (lhs, slot) = vm.current_call_frame.stack.pop1_and_top_mut_scalars()?;
        let rhs = *slot;
        let lhs_sign = lhs.bit(255);
        let rhs_sign = rhs.bit(255);

        *slot = match (lhs_sign, rhs_sign) {
            (false, true) => U256::one(),
            (true, false) => U256::zero(),
            #[expect(clippy::as_conversions, reason = "safe")]
            _ => (u256_lt(&rhs.0, &lhs.0) as u64).into(),
        };

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `EQ` opcode.
pub struct OpEqHandler;
impl OpcodeHandler for OpEqHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::EQ)?;

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut_scalars()?;
        #[expect(clippy::as_conversions, reason = "safe")]
        let res = u256_eq(&lhs.0, &rhs.0) as u64;
        *rhs = res.into();

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

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut_scalars()?;
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

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut_scalars()?;
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

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut_scalars()?;
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

        let (index, slot) = vm.current_call_frame.stack.pop1_and_top_mut_scalars()?;
        let value = *slot;
        *slot = match usize::try_from(index) {
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "x < 32 guard prevents overflow"
            )]
            Ok(x) if x < 32 => value.byte(31 - x).into(),
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

        let (shift_amount, slot) = vm.current_call_frame.stack.pop1_and_top_mut_scalars()?;
        let value = *slot;
        *slot = match u8::try_from(shift_amount) {
            #[expect(clippy::arithmetic_side_effects, reason = "U256 shift by u8 is safe")]
            Ok(shift_amount) => value << shift_amount,
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

        let (shift_amount, slot) = vm.current_call_frame.stack.pop1_and_top_mut_scalars()?;
        let value = *slot;
        *slot = match u8::try_from(shift_amount) {
            #[expect(clippy::arithmetic_side_effects, reason = "U256 shift by u8 is safe")]
            Ok(shift_amount) => value >> shift_amount,
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

        let (shift_amount, slot) = vm.current_call_frame.stack.pop1_and_top_mut_scalars()?;
        let value = *slot;
        #[expect(clippy::arithmetic_side_effects, reason = "U256 shift by u8 is safe")]
        {
            *slot = match (u8::try_from(shift_amount), value.bit(255)) {
                (Ok(shift_amount), false) => value >> shift_amount,
                (Ok(shift_amount), true) => !(!value >> shift_amount),
                (Err(_), false) => U256::zero(),
                (Err(_), true) => U256::MAX,
            };
        }

        Ok(OpcodeResult::Continue)
    }
}

#[cfg(test)]
mod u256_cmp_tests {
    use super::{u256_eq, u256_lt};
    use ethrex_common::U256;

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
            self.0 >> 16
        }
        fn u256(&mut self) -> U256 {
            // Bias toward equal high limbs to exercise short-circuit + tie paths.
            let hi = if self.next().is_multiple_of(3) {
                0
            } else {
                self.next()
            };
            U256([self.next(), self.next(), self.next(), hi])
        }
    }

    #[test]
    fn limb_cmp_matches_u256_operators() {
        let mut rng = Lcg(0x1234_5678_9abc_def0);
        for _ in 0..200_000 {
            let a = rng.u256();
            let b = rng.u256();
            assert_eq!(u256_lt(&a.0, &b.0), a < b, "lt {a:?} {b:?}");
            assert_eq!(u256_eq(&a.0, &b.0), a == b, "eq {a:?} {b:?}");
        }
        // Edge cases.
        for (a, b) in [
            (U256::zero(), U256::zero()),
            (U256::zero(), U256::one()),
            (U256::MAX, U256::MAX),
            (U256::MAX, U256::zero()),
            (U256([0, 0, 0, 1]), U256([u64::MAX, u64::MAX, u64::MAX, 0])),
        ] {
            assert_eq!(u256_lt(&a.0, &b.0), a < b);
            assert_eq!(u256_eq(&a.0, &b.0), a == b);
        }
    }
}
