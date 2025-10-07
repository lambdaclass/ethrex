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
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::LT)?;

        let [lhs, rhs] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame
            .stack
            .push1(((lhs < rhs) as u64).into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `GT` opcode.
pub struct OpGtHandler;
impl OpcodeHandler for OpGtHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::GT)?;

        let [lhs, rhs] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame
            .stack
            .push1(((lhs > rhs) as u64).into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SLT` opcode.
pub struct OpSLtHandler;
impl OpcodeHandler for OpSLtHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::SLT)?;

        let [lhs, rhs] = *vm.current_call_frame.stack.pop()?;
        let lhs_sign = lhs.bit(255);
        let rhs_sign = rhs.bit(255);

        vm.current_call_frame
            .stack
            .push1(match (lhs_sign, rhs_sign) {
                (false, true) => U256::zero(),
                (true, false) => U256::one(),
                _ => ((lhs < rhs) as u64).into(),
            })?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SGT` opcode.
pub struct OpSGtHandler;
impl OpcodeHandler for OpSGtHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::SGT)?;

        let [lhs, rhs] = *vm.current_call_frame.stack.pop()?;
        let lhs_sign = lhs.bit(255);
        let rhs_sign = rhs.bit(255);

        vm.current_call_frame
            .stack
            .push1(match (lhs_sign, rhs_sign) {
                (false, true) => U256::one(),
                (true, false) => U256::zero(),
                _ => ((lhs > rhs) as u64).into(),
            })?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `EQ` opcode.
pub struct OpEqHandler;
impl OpcodeHandler for OpEqHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::EQ)?;

        let [lhs, rhs] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame
            .stack
            .push1(((lhs == rhs) as u64).into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `ISZERO` opcode.
pub struct OpIsZeroHandler;
impl OpcodeHandler for OpIsZeroHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::ISZERO)?;

        let value = vm.current_call_frame.stack.pop1()?;
        vm.current_call_frame
            .stack
            .push1((value.is_zero() as u64).into())?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `AND` opcode.
pub struct OpAndHandler;
impl OpcodeHandler for OpAndHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::AND)?;

        let [lhs, rhs] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame.stack.push1(lhs & rhs)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `OR` opcode.
pub struct OpOrHandler;
impl OpcodeHandler for OpOrHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::OR)?;

        let [lhs, rhs] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame.stack.push1(lhs | rhs)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `XOR` opcode.
pub struct OpXorHandler;
impl OpcodeHandler for OpXorHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::XOR)?;

        let [lhs, rhs] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame.stack.push1(lhs ^ rhs)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `NOT` opcode.
pub struct OpNotHandler;
impl OpcodeHandler for OpNotHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::NOT)?;

        let value = vm.current_call_frame.stack.pop1()?;
        vm.current_call_frame.stack.push1(!value)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `BYTE` opcode.
pub struct OpByteHandler;
impl OpcodeHandler for OpByteHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame
            .increase_consumed_gas(gas_cost::BYTE)?;

        let [index, value] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame
            .stack
            .push1(match usize::try_from(index) {
                Ok(x) if x < 32 => value.byte(x).into(),
                _ => U256::zero(),
            })?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SHL` opcode.
pub struct OpShlHandler;
impl OpcodeHandler for OpShlHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::SHL)?;

        let [shift_amount, value] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame.stack.push1(value << shift_amount)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SHR` opcode.
pub struct OpShrHandler;
impl OpcodeHandler for OpShrHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::SHR)?;

        let [shift_amount, value] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame.stack.push1(value >> shift_amount)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Implementation for the `SAR` opcode.
pub struct OpSarHandler;
impl OpcodeHandler for OpSarHandler {
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        vm.current_call_frame.increase_consumed_gas(gas_cost::SAR)?;

        let [shift_amount, value] = *vm.current_call_frame.stack.pop()?;
        vm.current_call_frame.stack.push1(if value.bit(255) {
            !(!value >> shift_amount)
        } else {
            value >> shift_amount
        })?;

        Ok(OpcodeResult::Continue)
    }
}
