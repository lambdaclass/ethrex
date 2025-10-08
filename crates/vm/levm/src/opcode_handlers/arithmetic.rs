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
use std::cmp::Ordering;

/// Implementation for the `ADD` opcode.
pub struct OpAddHandler;
impl OpcodeHandler for OpAddHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::ADD)?;

        let [lhs, rhs] = *vm.current_call_frame.stack.pop()?;
        let (res, _) = lhs.overflowing_add(rhs);
        vm.current_call_frame.stack.push1(res)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SUB` opcode.
pub struct OpSubHandler;
impl OpcodeHandler for OpSubHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::SUB)?;

        let [lhs, rhs] = *vm.current_call_frame.stack.pop()?;
        let (res, _) = lhs.overflowing_sub(rhs);
        vm.current_call_frame.stack.push1(res)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `MUL` opcode.
pub struct OpMulHandler;
impl OpcodeHandler for OpMulHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::MUL)?;

        let [lhs, rhs] = *vm.current_call_frame.stack.pop()?;
        let (res, _) = lhs.overflowing_mul(rhs);
        vm.current_call_frame.stack.push1(res)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `DIV` opcode.
pub struct OpDivHandler;
impl OpcodeHandler for OpDivHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::DIV)?;

        let [lhs, rhs] = *vm.current_call_frame.stack.pop()?;
        match lhs.checked_div(rhs) {
            Some(res) => vm.current_call_frame.stack.push1(res)?,
            None => vm.current_call_frame.stack.push_zero()?,
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

        let [mut lhs, mut rhs] = *vm.current_call_frame.stack.pop()?;

        let mut sign = false;
        if lhs.bit(255) {
            lhs = U256::zero().overflowing_sub(lhs).0;
            sign = !sign;
        }
        if rhs.bit(255) {
            rhs = U256::zero().overflowing_sub(rhs).0;
            sign = !sign;
        }

        match lhs.checked_div(rhs) {
            Some(mut res) => {
                if sign {
                    res = U256::zero().overflowing_sub(res).0;
                }

                vm.current_call_frame.stack.push1(res)?
            }
            None => vm.current_call_frame.stack.push_zero()?,
        }

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `DIV` opcode.
pub struct OpModHandler;
impl OpcodeHandler for OpModHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::MOD)?;

        let [lhs, rhs] = *vm.current_call_frame.stack.pop()?;
        match lhs.checked_rem(rhs) {
            Some(res) => vm.current_call_frame.stack.push1(res)?,
            None => vm.current_call_frame.stack.push_zero()?,
        }

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SDIV` opcode.
pub struct OpSModHandler;
impl OpcodeHandler for OpSModHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::SMOD)?;

        let [mut lhs, mut rhs] = *vm.current_call_frame.stack.pop()?;

        let sign = lhs.bit(255);
        if sign {
            (lhs, _) = (!lhs).overflowing_add(U256::one());
        }
        if rhs.bit(255) {
            (rhs, _) = (!rhs).overflowing_add(U256::one());
        }

        match lhs.checked_rem(rhs) {
            Some(mut res) => {
                if sign {
                    (res, _) = (!res).overflowing_add(U256::one());
                }

                vm.current_call_frame.stack.push1(res)?
            }
            None => vm.current_call_frame.stack.push_zero()?,
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
            let (mut res, carry) = lhs.overflowing_add(rhs);

            // Increment the wrapped result only if the previous addition overflowed, and the modulo
            // is not a power of two.
            let is_mod_power_of_two = r#mod.0.into_iter().map(u64::count_ones).sum::<u32>() == 1;
            if carry && !is_mod_power_of_two {
                (res, _) = res.overflowing_add(U256::one());
            }

            res = match res.cmp(&r#mod) {
                Ordering::Less => res,
                Ordering::Equal => U256::zero(),
                Ordering::Greater if is_mod_power_of_two => res & (r#mod - 1),
                Ordering::Greater => res % r#mod,
            };

            vm.current_call_frame.stack.push1(res)?;
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

        let [lhs, rhs, r#mod] = *vm.current_call_frame.stack.pop()?;
        if lhs.is_zero() || rhs.is_zero() || r#mod.is_zero() {
            vm.current_call_frame.stack.push_zero()?;
        } else {
            let res = lhs.full_mul(rhs);

            let r#mod = r#mod.into();
            #[expect(clippy::unwrap_used, reason = "unreachable")]
            let res = match res.cmp(&r#mod) {
                Ordering::Less => res.try_into().unwrap(),
                Ordering::Equal => U256::zero(),
                Ordering::Greater => (res % r#mod).try_into().unwrap(),
            };

            vm.current_call_frame.stack.push1(res)?;
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
        vm.current_call_frame.stack.push1(res)?;

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

        let [index, mut value] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame
            .stack
            .push1(match usize::try_from(index) {
                Ok(x) if x < 32 => {
                    if value.bit(8 * x + 7) {
                        value |= U256::MAX << 8 * (x + 1);
                    } else if x != 31 {
                        value &= (U256::one() << 8 * (x + 1)) - 1;
                    }

                    value
                }
                _ => value,
            })?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `CLZ` opcode.
pub struct OpClzHandler;
impl OpcodeHandler for OpClzHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::CLZ)?;

        let value = vm.current_call_frame.stack.pop1()?;
        vm.current_call_frame
            .stack
            .push1(value.leading_zeros().into())?;

        Ok(OpcodeResult::Continue)
    }
}
