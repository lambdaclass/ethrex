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

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        #[expect(clippy::as_conversions, reason = "safe")]
        let res = (lhs < *rhs) as u64;
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

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        #[expect(clippy::as_conversions, reason = "safe")]
        let res = (lhs > *rhs) as u64;
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

        let (lhs, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        let rhs = *slot;
        let lhs_sign = lhs.bit(255);
        let rhs_sign = rhs.bit(255);

        *slot = match (lhs_sign, rhs_sign) {
            (false, true) => U256::zero(),
            (true, false) => U256::one(),
            #[expect(clippy::as_conversions, reason = "safe")]
            _ => ((lhs < rhs) as u64).into(),
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

        let (lhs, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        let rhs = *slot;
        let lhs_sign = lhs.bit(255);
        let rhs_sign = rhs.bit(255);

        *slot = match (lhs_sign, rhs_sign) {
            (false, true) => U256::one(),
            (true, false) => U256::zero(),
            #[expect(clippy::as_conversions, reason = "safe")]
            _ => ((lhs > rhs) as u64).into(),
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

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
        #[expect(clippy::as_conversions, reason = "safe")]
        let res = (lhs == *rhs) as u64;
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

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
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

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
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

        let (lhs, rhs) = vm.current_call_frame.stack.pop1_and_top_mut()?;
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

        let (index, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
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

        let (shift_amount, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
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

        let (shift_amount, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
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

        let (shift_amount, slot) = vm.current_call_frame.stack.pop1_and_top_mut()?;
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
