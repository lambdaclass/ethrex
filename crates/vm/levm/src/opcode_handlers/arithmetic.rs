use crate::{
    call_frame::CallFrame,
    errors::{InternalError, OpcodeSuccess, VMError},
    gas_cost,
    opcode_handlers::bitwise_comparison::checked_shift_left,
    vm::VM,
};
use ethrex_core::{U256, U512};

use super::bitwise_comparison::checked_shift_right;

// Arithmetic Operations (11)
// Opcodes: ADD, SUB, MUL, DIV, SDIV, MOD, SMOD, ADDMOD, MULMOD, EXP, SIGNEXTEND

impl VM {
    // ADD operation
    pub fn op_add(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::ADD)?;

        let augend = current_call_frame.stack.pop()?;
        let addend = current_call_frame.stack.pop()?;
        let sum = augend.overflowing_add(addend).0;
        current_call_frame.stack.push(sum)?;

        Ok(OpcodeSuccess::Continue)
    }

    // SUB operation
    pub fn op_sub(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::SUB)?;

        let minuend = current_call_frame.stack.pop()?;
        let subtrahend = current_call_frame.stack.pop()?;
        let difference = minuend.overflowing_sub(subtrahend).0;
        current_call_frame.stack.push(difference)?;

        Ok(OpcodeSuccess::Continue)
    }

    // MUL operation
    pub fn op_mul(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::MUL)?;

        let multiplicand = current_call_frame.stack.pop()?;
        let multiplier = current_call_frame.stack.pop()?;
        let product = multiplicand.overflowing_mul(multiplier).0;
        current_call_frame.stack.push(product)?;

        Ok(OpcodeSuccess::Continue)
    }

    // DIV operation
    pub fn op_div(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::DIV)?;

        let dividend = current_call_frame.stack.pop()?;
        let divisor = current_call_frame.stack.pop()?;
        let Some(quotient) = dividend.checked_div(divisor) else {
            current_call_frame.stack.push(U256::zero())?;
            return Ok(OpcodeSuccess::Continue);
        };
        current_call_frame.stack.push(quotient)?;

        Ok(OpcodeSuccess::Continue)
    }

    // SDIV operation
    pub fn op_sdiv(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::SDIV)?;

        let dividend = current_call_frame.stack.pop()?;
        let divisor = current_call_frame.stack.pop()?;
        if divisor.is_zero() || dividend.is_zero() {
            current_call_frame.stack.push(U256::zero())?;
            return Ok(OpcodeSuccess::Continue);
        }

        let abs_dividend = abs(dividend);
        let abs_divisor = abs(divisor);

        let quotient = match abs_dividend.checked_div(abs_divisor) {
            Some(quot) => {
                let quotient_is_negative = is_negative(dividend) ^ is_negative(divisor);
                if quotient_is_negative {
                    negate(quot)
                } else {
                    quot
                }
            }
            None => U256::zero(),
        };

        current_call_frame.stack.push(quotient)?;

        Ok(OpcodeSuccess::Continue)
    }

    // MOD operation
    pub fn op_mod(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::MOD)?;

        let dividend = current_call_frame.stack.pop()?;
        let divisor = current_call_frame.stack.pop()?;

        let remainder = dividend.checked_rem(divisor).unwrap_or_default();

        current_call_frame.stack.push(remainder)?;

        Ok(OpcodeSuccess::Continue)
    }

    // SMOD operation
    pub fn op_smod(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::SMOD)?;

        let unchecked_dividend = current_call_frame.stack.pop()?;
        let unchecked_divisor = current_call_frame.stack.pop()?;

        if unchecked_divisor.is_zero() || unchecked_dividend.is_zero() {
            current_call_frame.stack.push(U256::zero())?;
            return Ok(OpcodeSuccess::Continue);
        }

        let divisor = abs(unchecked_divisor);
        let dividend = abs(unchecked_dividend);

        let unchecked_remainder = match dividend.checked_rem(divisor) {
            Some(remainder) => remainder,
            None => {
                current_call_frame.stack.push(U256::zero())?;
                return Ok(OpcodeSuccess::Continue);
            }
        };

        let remainder = if is_negative(unchecked_dividend) {
            negate(unchecked_remainder)
        } else {
            unchecked_remainder
        };

        current_call_frame.stack.push(remainder)?;

        Ok(OpcodeSuccess::Continue)
    }

    // ADDMOD operation
    pub fn op_addmod(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::ADDMOD)?;

        let augend = current_call_frame.stack.pop()?;
        let addend = current_call_frame.stack.pop()?;
        let modulus = current_call_frame.stack.pop()?;

        if modulus.is_zero() {
            current_call_frame.stack.push(U256::zero())?;
            return Ok(OpcodeSuccess::Continue);
        }

        let new_augend: U512 = augend.into();
        let new_addend: U512 = addend.into();

        let sum = new_augend.checked_add(new_addend).ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;

        let sum_mod = sum
            .checked_rem(modulus.into())
            .ok_or(VMError::Internal(
                InternalError::ArithmeticOperationOverflow,
            ))?
            .try_into()
            .map_err(|_err| VMError::Internal(InternalError::ArithmeticOperationOverflow))?;

        current_call_frame.stack.push(sum_mod)?;

        Ok(OpcodeSuccess::Continue)
    }

    // MULMOD operation
    pub fn op_mulmod(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::MULMOD)?;

        let multiplicand = current_call_frame.stack.pop()?;
        let multiplier = current_call_frame.stack.pop()?;
        let modulus = current_call_frame.stack.pop()?;

        if modulus.is_zero() || multiplicand.is_zero() || multiplier.is_zero() {
            current_call_frame.stack.push(U256::zero())?;
            return Ok(OpcodeSuccess::Continue);
        }

        let multiplicand: U512 = multiplicand.into();
        let multiplier: U512 = multiplier.into();

        let product = multiplicand
            .checked_mul(multiplier)
            .ok_or(VMError::Internal(
                InternalError::ArithmeticOperationOverflow,
            ))?;
        let product_mod: U256 = product
            .checked_rem(modulus.into())
            .ok_or(VMError::Internal(
                InternalError::ArithmeticOperationOverflow,
            ))?
            .try_into()
            .map_err(|_err| VMError::Internal(InternalError::ArithmeticOperationOverflow))?;

        current_call_frame.stack.push(product_mod)?;

        Ok(OpcodeSuccess::Continue)
    }

    // EXP operation
    pub fn op_exp(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeSuccess, VMError> {
        let base = current_call_frame.stack.pop()?;
        let exponent = current_call_frame.stack.pop()?;

        let gas_cost = gas_cost::exp(exponent)?;

        self.increase_consumed_gas(current_call_frame, gas_cost)?;

        let power = base.overflowing_pow(exponent).0;
        current_call_frame.stack.push(power)?;

        Ok(OpcodeSuccess::Continue)
    }

    // SIGNEXTEND operation
    pub fn op_signextend(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        self.increase_consumed_gas(current_call_frame, gas_cost::SIGNEXTEND)?;

        let byte_size_minus_one = current_call_frame.stack.pop()?;
        let value_to_extend = current_call_frame.stack.pop()?;

        if byte_size_minus_one > U256::from(31) {
            current_call_frame.stack.push(value_to_extend)?;
            return Ok(OpcodeSuccess::Continue);
        }

        let bits_per_byte = U256::from(8);
        let sign_bit_position_on_byte = U256::from(7);

        let sign_bit_index = bits_per_byte
            .checked_mul(byte_size_minus_one)
            .and_then(|total_bits| total_bits.checked_add(sign_bit_position_on_byte))
            .ok_or(VMError::Internal(
                InternalError::ArithmeticOperationOverflow,
            ))?;

        let shifted_value = checked_shift_right(value_to_extend, sign_bit_index)?;
        let sign_bit = shifted_value & U256::one();

        let sign_bit_mask = checked_shift_left(U256::one(), sign_bit_index)?
            .checked_sub(U256::one())
            .ok_or(VMError::Internal(
                InternalError::ArithmeticOperationUnderflow,
            ))?; //Shifted should be at least one

        let result = if sign_bit.is_zero() {
            value_to_extend & sign_bit_mask
        } else {
            value_to_extend | !sign_bit_mask
        };
        current_call_frame.stack.push(result)?;

        Ok(OpcodeSuccess::Continue)
    }
}

/// Shifts the value to the right by 255 bits and checks the most significant bit is a 1
fn is_negative(value: U256) -> bool {
    value.bit(255)
}

/// Negates a number in two's complement
fn negate(value: U256) -> U256 {
    let (dividend, _overflowed) = (!value).overflowing_add(U256::one());
    dividend
}

fn abs(value: U256) -> U256 {
    if is_negative(value) {
        negate(value)
    } else {
        value
    }
}
