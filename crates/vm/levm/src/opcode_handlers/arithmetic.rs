use std::cell::OnceCell;

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
    pub fn op_add(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::ADD) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [augend, addend] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let sum = augend.overflowing_add(addend).0;

        if let Err(err) = self.current_call_frame.stack.push1(sum) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // SUB operation
    pub fn op_sub(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::SUB) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [minuend, subtrahend] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let difference = minuend.overflowing_sub(subtrahend).0;

        if let Err(err) = self.current_call_frame.stack.push1(difference) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // MUL operation
    pub fn op_mul(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::MUL) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [multiplicand, multiplier] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let product = multiplicand.overflowing_mul(multiplier).0;

        if let Err(err) = self.current_call_frame.stack.push1(product) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // DIV operation
    pub fn op_div(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::DIV) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [dividend, divisor] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let Some(quotient) = dividend.checked_div(divisor) else {
            if let Err(err) = self.current_call_frame.stack.push_zero() {
                error.set(err.into());
                return OpcodeResult::Halt;
            }

            return OpcodeResult::Continue;
        };
        if let Err(err) = self.current_call_frame.stack.push1(quotient) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // SDIV operation
    pub fn op_sdiv(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::SDIV)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [dividend, divisor] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        if divisor.is_zero() || dividend.is_zero() {
            if let Err(err) = self.current_call_frame.stack.push_zero() {
                error.set(err.into());
                return OpcodeResult::Halt;
            };
            return OpcodeResult::Continue;
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

        if let Err(err) = self.current_call_frame.stack.push1(quotient) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // MOD operation
    pub fn op_mod(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::MOD) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [dividend, divisor] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let remainder = dividend.checked_rem(divisor).unwrap_or_default();

        if let Err(err) = self.current_call_frame.stack.push1(remainder) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // SMOD operation
    pub fn op_smod(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::SMOD)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [unchecked_dividend, unchecked_divisor] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if unchecked_divisor.is_zero() || unchecked_dividend.is_zero() {
            if let Err(err) = self.current_call_frame.stack.push_zero() {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
            return OpcodeResult::Continue;
        }

        let divisor = abs(unchecked_divisor);
        let dividend = abs(unchecked_dividend);

        let unchecked_remainder = match dividend.checked_rem(divisor) {
            Some(remainder) => remainder,
            None => {
                if let Err(err) = self.current_call_frame.stack.push_zero() {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
                return OpcodeResult::Continue;
            }
        };

        let remainder = if is_negative(unchecked_dividend) {
            negate(unchecked_remainder)
        } else {
            unchecked_remainder
        };

        if let Err(err) = self.current_call_frame.stack.push1(remainder) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // ADDMOD operation
    pub fn op_addmod(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::ADDMOD)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [augend, addend, modulus] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if modulus.is_zero() {
            if let Err(err) = self.current_call_frame.stack.push_zero() {
                error.set(err.into());
                return OpcodeResult::Halt;
            };
            return OpcodeResult::Continue;
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

        if let Err(err) = self.current_call_frame.stack.push1(sum_mod) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // MULMOD operation
    pub fn op_mulmod(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::MULMOD)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [multiplicand, multiplier, modulus] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if modulus.is_zero() || multiplicand.is_zero() || multiplier.is_zero() {
            if let Err(err) = self.current_call_frame.stack.push_zero() {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
            return OpcodeResult::Continue;
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

        if let Err(err) = self.current_call_frame.stack.push1(product_mod) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // EXP operation
    pub fn op_exp(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        let [base, exponent] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        let gas_cost = match gas_cost::exp(exponent) {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let power = base.overflowing_pow(exponent).0;
        if let Err(err) = self.current_call_frame.stack.push1(power) {
            error.set(err.into());
            return OpcodeResult::Halt;
        };

        OpcodeResult::Continue
    }

    // SIGNEXTEND operation
    pub fn op_signextend(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::SIGNEXTEND)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [byte_size_minus_one, value_to_extend] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if byte_size_minus_one > U256::from(31) {
            if let Err(err) = self.current_call_frame.stack.push1(value_to_extend) {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
            return OpcodeResult::Continue;
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

            if let Err(err) = self.current_call_frame.stack.push1(result) {
                error.set(err.into());
                return OpcodeResult::Halt;
            }

            OpcodeResult::Continue
        }
    }

    pub fn op_clz(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::CLZ) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let value = match self.current_call_frame.stack.pop1() {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if let Err(err) = self
            .current_call_frame
            .stack
            .push1(U256::from(value.leading_zeros()))
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
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
