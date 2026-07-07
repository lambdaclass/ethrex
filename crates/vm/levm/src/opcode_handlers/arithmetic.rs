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
#[cfg(not(feature = "zisk"))]
use ethrex_common::U512;

#[inline(always)]
fn negate_u256(x: U256) -> U256 {
    #[cfg(feature = "zisk")]
    {
        crate::zisk_u256::overflowing_sub(U256::zero(), x).0
    }
    #[cfg(not(feature = "zisk"))]
    {
        U256::zero().overflowing_sub(x).0
    }
}

/// Implementation for the `ADD` opcode.
pub struct OpAddHandler;
impl OpcodeHandler for OpAddHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::ADD)?;

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        #[cfg(feature = "zisk")]
        {
            *rhs = crate::zisk_u256::overflowing_add(lhs, *rhs).0;
        }
        #[cfg(not(feature = "zisk"))]
        {
            *rhs = lhs.overflowing_add(*rhs).0;
        }

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
        #[cfg(feature = "zisk")]
        {
            *rhs = crate::zisk_u256::overflowing_sub(lhs, *rhs).0;
        }
        #[cfg(not(feature = "zisk"))]
        {
            *rhs = lhs.overflowing_sub(*rhs).0;
        }

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
        #[cfg(feature = "zisk")]
        {
            *rhs = crate::zisk_u256::wrapping_mul(lhs, *rhs);
        }
        #[cfg(not(feature = "zisk"))]
        {
            *rhs = lhs.overflowing_mul(*rhs).0;
        }

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
        #[cfg(feature = "zisk")]
        {
            *rhs = crate::zisk_u256::checked_div(lhs, *rhs);
        }
        #[cfg(not(feature = "zisk"))]
        {
            *rhs = lhs.checked_div(*rhs).unwrap_or(U256::zero());
        }

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
            lhs = negate_u256(lhs);
            sign = !sign;
        }
        if rhs.bit(255) {
            rhs = negate_u256(rhs);
            sign = !sign;
        }

        #[cfg(feature = "zisk")]
        {
            let res = crate::zisk_u256::checked_div(lhs, rhs);
            *slot = if sign && !res.is_zero() {
                negate_u256(res)
            } else {
                res
            };
        }
        #[cfg(not(feature = "zisk"))]
        {
            *slot = match lhs.checked_div(rhs) {
                Some(mut res) => {
                    if sign {
                        res = negate_u256(res);
                    }
                    res
                }
                None => U256::zero(),
            };
        }

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
        #[cfg(feature = "zisk")]
        {
            *rhs = crate::zisk_u256::checked_rem(lhs, *rhs);
        }
        #[cfg(not(feature = "zisk"))]
        {
            *rhs = lhs.checked_rem(*rhs).unwrap_or(U256::zero());
        }

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
            lhs = negate_u256(lhs);
        }
        if rhs.bit(255) {
            rhs = negate_u256(rhs);
        }

        #[cfg(feature = "zisk")]
        {
            let res = crate::zisk_u256::checked_rem(lhs, rhs);
            *slot = if sign && !res.is_zero() {
                negate_u256(res)
            } else {
                res
            };
        }
        #[cfg(not(feature = "zisk"))]
        {
            *slot = match lhs.checked_rem(rhs) {
                Some(mut res) => {
                    if sign {
                        res = negate_u256(res);
                    }
                    res
                }
                None => U256::zero(),
            };
        }

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
        if r#mod.is_zero() || r#mod == U256::one() {
            vm.current_call_frame.stack.push_zero()?;
        } else {
            #[cfg(feature = "zisk")]
            {
                let res = crate::zisk_u256::addmod(lhs, rhs, r#mod);
                vm.current_call_frame.stack.push(res)?;
            }
            #[cfg(not(feature = "zisk"))]
            {
                #[expect(
                    clippy::arithmetic_side_effects,
                    reason = "mod is checked non-zero above"
                )]
                let res = U512::from(lhs).overflowing_add(rhs.into()).0 % r#mod;
                vm.current_call_frame
                    .stack
                    .push(U256([res.0[0], res.0[1], res.0[2], res.0[3]]))?;
            }
        }

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
            #[cfg(feature = "zisk")]
            {
                let res = crate::zisk_u256::mulmod(multiplicand, multiplier, modulus);
                vm.current_call_frame.stack.push(res)?;
            }
            #[cfg(not(feature = "zisk"))]
            {
                let a_bytes = multiplicand.to_big_endian();
                let b_bytes = multiplier.to_big_endian();
                let m_bytes = modulus.to_big_endian();
                let result_bytes = vm.crypto.mulmod256(&a_bytes, &b_bytes, &m_bytes);
                let product_mod = U256::from_big_endian(&result_bytes);
                vm.current_call_frame.stack.push(product_mod)?;
            }
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

        let (res, _) = base.overflowing_pow(exp);
        vm.current_call_frame.stack.push(res)?;

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
        let mut value = *slot;
        *slot = match usize::try_from(index) {
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "x < 32 guard prevents overflow"
            )]
            Ok(x) if x < 32 => {
                if value.bit(8 * x + 7) {
                    value |= U256::MAX << (8 * (x + 1));
                } else if x != 31 {
                    value &= (U256::one() << (8 * (x + 1))) - 1;
                }

                value
            }
            _ => value,
        };

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
