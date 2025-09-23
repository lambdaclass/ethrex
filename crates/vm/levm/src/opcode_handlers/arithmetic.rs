use crate::{
    errors::{OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};
use ethrex_common::{U256, U512};

// Arithmetic Operations (11)
// Opcodes: ADD, SUB, MUL, DIV, SDIV, MOD, SMOD, ADDMOD, MULMOD, EXP, SIGNEXTEND

impl<'a> VM<'a> {
    // ADD operation
    pub fn op_add(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::ADD)?;

        let [augend, addend] = *current_call_frame.stack.pop()?;
        let sum = augend.overflowing_add(addend).0;
        current_call_frame.stack.push1(sum)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SUB operation
    pub fn op_sub(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::SUB)?;

        let [minuend, subtrahend] = *current_call_frame.stack.pop()?;
        let difference = minuend.overflowing_sub(subtrahend).0;
        current_call_frame.stack.push1(difference)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // MUL operation
    pub fn op_mul(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::MUL)?;

        let [multiplicand, multiplier] = *current_call_frame.stack.pop()?;
        let product = multiplicand.overflowing_mul(multiplier).0;
        current_call_frame.stack.push1(product)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // DIV operation
    pub fn op_div(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::DIV)?;

        let [dividend, divisor] = *current_call_frame.stack.pop()?;
        let Some(quotient) = dividend.checked_div(divisor) else {
            current_call_frame.stack.push_zero()?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        };
        current_call_frame.stack.push1(quotient)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SDIV operation
    pub fn op_sdiv(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::SDIV)?;

        let [dividend, divisor] = *current_call_frame.stack.pop()?;
        if divisor.is_zero() || dividend.is_zero() {
            current_call_frame.stack.push_zero()?;
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
            None => U256::zero(),
        };

        current_call_frame.stack.push1(quotient)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // MOD operation
    pub fn op_mod(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::MOD)?;

        let [dividend, divisor] = *current_call_frame.stack.pop()?;

        let remainder = dividend.checked_rem(divisor).unwrap_or_default();

        current_call_frame.stack.push1(remainder)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SMOD operation
    pub fn op_smod(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::SMOD)?;

        let [unchecked_dividend, unchecked_divisor] = *current_call_frame.stack.pop()?;

        if unchecked_divisor.is_zero() || unchecked_dividend.is_zero() {
            current_call_frame.stack.push_zero()?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let divisor = abs(unchecked_divisor);
        let dividend = abs(unchecked_dividend);

        let unchecked_remainder = match dividend.checked_rem(divisor) {
            Some(remainder) => remainder,
            None => {
                current_call_frame.stack.push_zero()?;
                return Ok(OpcodeResult::Continue { pc_increment: 1 });
            }
        };

        let remainder = if is_negative(unchecked_dividend) {
            negate(unchecked_remainder)
        } else {
            unchecked_remainder
        };

        current_call_frame.stack.push1(remainder)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // ADDMOD operation
    pub fn op_addmod(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::ADDMOD)?;

        let [augend, addend, modulus] = *current_call_frame.stack.pop()?;

        if modulus.is_zero() {
            current_call_frame.stack.push_zero()?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let new_augend: U512 = augend.into();
        let new_addend: U512 = addend.into();

        #[allow(
            clippy::arithmetic_side_effects,
            reason = "both values come from a u256, so the product can fit in a U512"
        )]
        let sum = new_augend + new_addend;
        #[allow(
            clippy::arithmetic_side_effects,
            reason = "can't trap because non-zero modulus"
        )]
        let sum_mod = sum % modulus;

        #[allow(clippy::expect_used, reason = "can't overflow")]
        let sum_mod: U256 = sum_mod
            .try_into()
            .expect("can't fail because we applied % mod where mod is a U256 value");

        current_call_frame.stack.push1(sum_mod)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // MULMOD operation
    pub fn op_mulmod(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::MULMOD)?;

        let [multiplicand, multiplier, modulus] = *current_call_frame.stack.pop()?;

        if modulus.is_zero() || multiplicand.is_zero() || multiplier.is_zero() {
            current_call_frame.stack.push_zero()?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        let multiplicand: U512 = multiplicand.into();
        let multiplier: U512 = multiplier.into();

        #[allow(
            clippy::arithmetic_side_effects,
            reason = "both values come from a u256, so the product can fit in a U512"
        )]
        let product = multiplicand * multiplier;
        #[allow(clippy::arithmetic_side_effects, reason = "can't overflow")]
        let product_mod = product % modulus;

        #[allow(clippy::expect_used, reason = "can't overflow")]
        let product_mod: U256 = product_mod
            .try_into()
            .expect("can't fail because we applied % mod where mod is a U256 value");

        current_call_frame.stack.push1(product_mod)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // EXP operation
    pub fn op_exp(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        let [base, exponent] = *current_call_frame.stack.pop()?;

        let gas_cost = gas_cost::exp(exponent)?;

        current_call_frame.increase_consumed_gas(gas_cost)?;

        let power = base.overflowing_pow(exponent).0;
        current_call_frame.stack.push1(power)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SIGNEXTEND operation
    pub fn op_signextend(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::SIGNEXTEND)?;

        let [byte_size_minus_one, value_to_extend] = *current_call_frame.stack.pop()?;

        if byte_size_minus_one > U256::from(31) {
            current_call_frame.stack.push1(value_to_extend)?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        #[allow(
            clippy::arithmetic_side_effects,
            reason = "Since byte_size_minus_one ≤ 31, overflow is impossible"
        )]
        let sign_bit_index = byte_size_minus_one * 8 + 7;

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "sign_bit_index max value is 31 * 8 + 7 = 255, which can't overflow."
        )]
        {
            let sign_bit = (value_to_extend >> sign_bit_index) & U256::one();
            let mask = (U256::one() << sign_bit_index) - U256::one();

            let result = if sign_bit.is_zero() {
                value_to_extend & mask
            } else {
                value_to_extend | !mask
            };

            current_call_frame.stack.push1(result)?;

            Ok(OpcodeResult::Continue { pc_increment: 1 })
        }
    }

    pub fn op_clz(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::CLZ)?;

        let value = self.current_call_frame.stack.pop1()?;

        self.current_call_frame
            .stack
            .push1(U256::from(value.leading_zeros()))?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
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
