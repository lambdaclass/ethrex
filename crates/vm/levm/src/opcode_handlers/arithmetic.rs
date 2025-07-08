use crate::{
    errors::{InternalError, OpcodeResult, VMError},
    gas_cost,
    opcode_handlers::bitwise_comparison::checked_shift_left,
    vm::VM,
};
use ethrex_common::{
    U256, U512,
    utils::{u256_from_u512, u256_overflowing_pow, u512_from_u256},
};

// Arithmetic Operations (11)
// Opcodes: ADD, SUB, MUL, DIV, SDIV, MOD, SMOD, ADDMOD, MULMOD, EXP, SIGNEXTEND

impl<'a> VM<'a> {
    // ADD operation
    pub fn op_add(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::ADD)?;

        let [augend, addend] = *current_call_frame.stack.pop()?;
        let sum = augend.overflowing_add(addend).0;
        current_call_frame.stack.push(&[sum])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SUB operation
    pub fn op_sub(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::SUB)?;

        let [minuend, subtrahend] = *current_call_frame.stack.pop()?;
        let difference = minuend.overflowing_sub(subtrahend).0;
        current_call_frame.stack.push(&[difference])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // MUL operation
    pub fn op_mul(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::MUL)?;

        let [multiplicand, multiplier] = *current_call_frame.stack.pop()?;
        let product = multiplicand.overflowing_mul(multiplier).0;
        current_call_frame.stack.push(&[product])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // DIV operation
    pub fn op_div(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::DIV)?;

        let [dividend, divisor] = *current_call_frame.stack.pop()?;
        let Some(quotient) = dividend.checked_div(divisor) else {
            current_call_frame.stack.push(&[U256::ZERO])?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        };
        current_call_frame.stack.push(&[quotient])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SDIV operation
    pub fn op_sdiv(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::SDIV)?;

        let [dividend, divisor] = *current_call_frame.stack.pop()?;
        if divisor == U256::ZERO || dividend == U256::ZERO {
            current_call_frame.stack.push(&[U256::ZERO])?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
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
            None => U256::ZERO,
        };

        current_call_frame.stack.push(&[quotient])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // MOD operation
    pub fn op_mod(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::MOD)?;

        let [dividend, divisor] = *current_call_frame.stack.pop()?;

        let remainder = dividend.checked_rem(divisor).unwrap_or_default();

        current_call_frame.stack.push(&[remainder])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SMOD operation
    pub fn op_smod(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::SMOD)?;

        let [unchecked_dividend, unchecked_divisor] = *current_call_frame.stack.pop()?;

        if unchecked_divisor == U256::ZERO || unchecked_dividend == U256::ZERO {
            current_call_frame.stack.push(&[U256::ZERO])?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let divisor = abs(unchecked_divisor);
        let dividend = abs(unchecked_dividend);

        let unchecked_remainder = match dividend.checked_rem(divisor) {
            Some(remainder) => remainder,
            None => {
                current_call_frame.stack.push(&[U256::ZERO])?;
                return Ok(OpcodeResult::Continue { pc_increment: 1 });
            }
        };

        let remainder = if is_negative(unchecked_dividend) {
            negate(unchecked_remainder)
        } else {
            unchecked_remainder
        };

        current_call_frame.stack.push(&[remainder])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // ADDMOD operation
    pub fn op_addmod(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::ADDMOD)?;

        let [augend, addend, modulus] = *current_call_frame.stack.pop()?;

        if modulus == U256::ZERO {
            current_call_frame.stack.push(&[U256::ZERO])?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let new_augend: U512 = u512_from_u256(augend);
        let new_addend: U512 = u512_from_u256(addend);

        let sum = new_augend
            .checked_add(new_addend)
            .ok_or(InternalError::Overflow)?;

        let sum_mod = u256_from_u512(
            sum.checked_rem(u512_from_u256(modulus))
                .ok_or(InternalError::Overflow)?,
        )
        .map_err(|_| InternalError::Overflow)?;

        current_call_frame.stack.push(&[sum_mod])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // MULMOD operation
    pub fn op_mulmod(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::MULMOD)?;

        let [multiplicand, multiplier, modulus] = *current_call_frame.stack.pop()?;

        if modulus == U256::ZERO || multiplicand == U256::ZERO || multiplier == U256::ZERO {
            current_call_frame.stack.push(&[U256::ZERO])?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let multiplicand: U512 = u512_from_u256(multiplicand);
        let multiplier: U512 = u512_from_u256(multiplier);

        let product = multiplicand
            .checked_mul(multiplier)
            .ok_or(InternalError::Overflow)?;
        let product_mod = product
            .checked_rem(u512_from_u256(modulus))
            .ok_or(InternalError::Overflow)?;

        let product_mod: U256 = u256_from_u512(product_mod).map_err(|_| InternalError::Overflow)?;

        current_call_frame.stack.push(&[product_mod])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // EXP operation
    pub fn op_exp(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        let [base, exponent] = *current_call_frame.stack.pop()?;

        let gas_cost = gas_cost::exp(exponent)?;

        current_call_frame.increase_consumed_gas(gas_cost)?;

        let power = u256_overflowing_pow(base, exponent).0;
        current_call_frame.stack.push(&[power])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SIGNEXTEND operation
    pub fn op_signextend(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = self.current_call_frame_mut()?;
        current_call_frame.increase_consumed_gas(gas_cost::SIGNEXTEND)?;

        let [byte_size_minus_one, value_to_extend] = *current_call_frame.stack.pop()?;

        if byte_size_minus_one > U256::from(31u32) {
            current_call_frame.stack.push(&[value_to_extend])?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let bits_per_byte = U256::from(8u32);
        let sign_bit_position_on_byte = U256::from(7u32);

        let sign_bit_index = bits_per_byte
            .checked_mul(byte_size_minus_one)
            .and_then(|total_bits| total_bits.checked_add(sign_bit_position_on_byte))
            .ok_or(InternalError::Overflow)?;

        #[expect(clippy::arithmetic_side_effects)]
        let shifted_value = value_to_extend >> sign_bit_index;
        let sign_bit = shifted_value & U256::ONE;

        let sign_bit_mask = checked_shift_left(U256::ONE, sign_bit_index)?
            .checked_sub(U256::ONE)
            .ok_or(InternalError::Underflow)?; //Shifted should be at least one

        let result = if sign_bit == U256::ZERO {
            value_to_extend & sign_bit_mask
        } else {
            value_to_extend | !sign_bit_mask
        };
        current_call_frame.stack.push(&[result])?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}

/// Checks its negative by checking leading zeros is 0
#[allow(clippy::arithmetic_side_effects)]
pub fn is_negative(value: U256) -> bool {
    (value >> 255) == U256::ONE
}

/// Negates a number in two's complement
fn negate(value: U256) -> U256 {
    let (dividend, _overflowed) = (!value).overflowing_add(U256::ONE);
    dividend
}

fn abs(value: U256) -> U256 {
    if is_negative(value) {
        negate(value)
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::as_conversions)]
    use ethrex_common::U256;

    use crate::opcode_handlers::arithmetic::{abs, is_negative, negate};

    #[test]
    fn test_is_negative() {
        let value: U256 = U256::new((-1i128) as u128) << 128;
        assert!(is_negative(value));
        assert!(!is_negative(abs(value)));
        assert!(is_negative(negate(U256::ONE)));
    }
}
