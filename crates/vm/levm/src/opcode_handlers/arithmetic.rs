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
use ethrex_common::{U256, U512};
use std::cmp::Ordering;

/// Implementation for the `ADD` opcode.
pub struct OpAddHandler;
impl OpcodeHandler for OpAddHandler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::ADD)?;

        let [lhs, rhs] = *vm.current_call_frame.stack.pop()?;
        let (res, _) = lhs.overflowing_add(rhs);
        vm.current_call_frame.stack.push(res)?;

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
        vm.current_call_frame.stack.push(res)?;

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
        vm.current_call_frame.stack.push(res)?;

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
            Some(res) => vm.current_call_frame.stack.push(res)?,
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

                vm.current_call_frame.stack.push(res)?
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
            Some(res) => vm.current_call_frame.stack.push(res)?,
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

                vm.current_call_frame.stack.push(res)?
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
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "mod is checked non-zero above"
            )]
            let res = U512::from(lhs).overflowing_add(rhs.into()).0 % r#mod;
            vm.current_call_frame
                .stack
                .push(U256([res.0[0], res.0[1], res.0[2], res.0[3]]))?;
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
            #[cfg(not(feature = "zisk"))]
            let res = {
                let res = lhs.full_mul(rhs);

                let r#mod = r#mod.into();
                #[expect(clippy::unwrap_used, reason = "unreachable")]
                match res.cmp(&r#mod) {
                    Ordering::Less => res.try_into().unwrap(),
                    Ordering::Equal => U256::zero(),
                    #[expect(
                        clippy::arithmetic_side_effects,
                        reason = "mod is checked non-zero above"
                    )]
                    Ordering::Greater => (res % r#mod).try_into().unwrap(),
                }
            };

            #[cfg(feature = "zisk")]
            let res = unsafe {
                use std::mem::MaybeUninit;
                use ziskos::zisklib::mulmod256_c;

                let res = MaybeUninit::<[u64; 4]>::uninit();
                mulmod256_c(
                    lhs.0.as_ptr(),
                    rhs.0.as_ptr(),
                    r#mod.0.as_ptr(),
                    res.as_mut_ptr(),
                );
                U256(res.assume_init())
            };

            vm.current_call_frame.stack.push(res)?;
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

        let [index, mut value] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame
            .stack
            .push(match usize::try_from(index) {
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
            .push(value.leading_zeros().into())?;

        Ok(OpcodeResult::Continue)
    }
}
