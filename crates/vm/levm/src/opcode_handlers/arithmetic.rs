use crate::{
    U256,
    errors::{OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};

// Arithmetic Operations (11)
// Opcodes: ADD, SUB, MUL, DIV, SDIV, MOD, SMOD, ADDMOD, MULMOD, EXP, SIGNEXTEND

impl<'a> VM<'a> {
    // ADD operation
    #[inline]
    pub fn op_add(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::ADD)?;

        let [augend, addend] = *current_call_frame.stack.pop()?;
        let sum = augend.overflowing_add(addend).0;
        current_call_frame.stack.push(sum)?;

        Ok(OpcodeResult::Continue)
    }

    // SUB operation
    pub fn op_sub(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::SUB)?;

        let [minuend, subtrahend] = *current_call_frame.stack.pop()?;
        let difference = minuend.overflowing_sub(subtrahend).0;
        current_call_frame.stack.push(difference)?;

        Ok(OpcodeResult::Continue)
    }

    // MUL operation
    pub fn op_mul(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::MUL)?;

        let [multiplicand, multiplier] = *current_call_frame.stack.pop()?;
        let product = multiplicand.overflowing_mul(multiplier).0;
        current_call_frame.stack.push(product)?;

        Ok(OpcodeResult::Continue)
    }

    // DIV operation
    pub fn op_div(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::DIV)?;

        let [dividend, divisor] = *current_call_frame.stack.pop()?;
        let Some(quotient) = dividend.checked_div(divisor) else {
            current_call_frame.stack.push_zero()?;
            return Ok(OpcodeResult::Continue);
        };
        current_call_frame.stack.push(quotient)?;

        Ok(OpcodeResult::Continue)
    }

    // SDIV operation
    pub fn op_sdiv(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::SDIV)?;

        let [dividend, divisor] = *current_call_frame.stack.pop()?;
        if divisor.is_zero() || dividend.is_zero() {
            current_call_frame.stack.push_zero()?;
            return Ok(OpcodeResult::Continue);
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

        current_call_frame.stack.push(quotient)?;

        Ok(OpcodeResult::Continue)
    }

    // MOD operation
    pub fn op_mod(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::MOD)?;

        let [dividend, divisor] = *current_call_frame.stack.pop()?;

        let remainder = dividend.checked_rem(divisor).unwrap_or_default();

        current_call_frame.stack.push(remainder)?;

        Ok(OpcodeResult::Continue)
    }

    // SMOD operation
    pub fn op_smod(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::SMOD)?;

        let [unchecked_dividend, unchecked_divisor] = *current_call_frame.stack.pop()?;

        if unchecked_divisor.is_zero() || unchecked_dividend.is_zero() {
            current_call_frame.stack.push_zero()?;
            return Ok(OpcodeResult::Continue);
        }

        let divisor = abs(unchecked_divisor);
        let dividend = abs(unchecked_dividend);

        let unchecked_remainder = match dividend.checked_rem(divisor) {
            Some(remainder) => remainder,
            None => {
                current_call_frame.stack.push_zero()?;
                return Ok(OpcodeResult::Continue);
            }
        };

        let remainder = if is_negative(unchecked_dividend) {
            negate(unchecked_remainder)
        } else {
            unchecked_remainder
        };

        current_call_frame.stack.push(remainder)?;

        Ok(OpcodeResult::Continue)
    }

    // ADDMOD operation
    pub fn op_addmod(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::ADDMOD)?;

        let [augend, addend, modulus] = *current_call_frame.stack.pop()?;

        if modulus.is_zero() {
            current_call_frame.stack.push_zero()?;
            return Ok(OpcodeResult::Continue);
        }

        let sum_mod = augend.add_mod(addend, modulus);

        current_call_frame.stack.push(sum_mod)?;

        Ok(OpcodeResult::Continue)
    }

    // MULMOD operation
    pub fn op_mulmod(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::MULMOD)?;

        let [multiplicand, multiplier, modulus] = *current_call_frame.stack.pop()?;

        if modulus.is_zero() || multiplicand.is_zero() || multiplier.is_zero() {
            current_call_frame.stack.push_zero()?;
            return Ok(OpcodeResult::Continue);
        }

        #[cfg(feature = "zisk")]
        let product_mod = {
            use ziskos::zisklib::mulmod256_c;
            let mut product_mod = U256::ZERO;
            #[expect(unsafe_code)]
            unsafe {
                mulmod256_c(
                    multiplicand.as_limbs().as_ptr(),
                    multiplier.as_limbs().as_ptr(),
                    modulus.as_limbs().as_ptr(),
                    product_mod.as_limbs_mut().as_mut_ptr(),
                );
            }
            product_mod
        };

        #[cfg(not(feature = "zisk"))]
        let product_mod = multiplicand.mul_mod(multiplier, modulus);

        current_call_frame.stack.push(product_mod)?;

        Ok(OpcodeResult::Continue)
    }

    // EXP operation
    pub fn op_exp(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        let [base, exponent] = *current_call_frame.stack.pop()?;

        let gas_cost = gas_cost::exp(exponent)?;

        current_call_frame.increase_consumed_gas(gas_cost)?;

        let power = base.overflowing_pow(exponent).0;
        current_call_frame.stack.push(power)?;

        Ok(OpcodeResult::Continue)
    }

    // SIGNEXTEND operation
    pub fn op_signextend(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::SIGNEXTEND)?;

        let [byte_size_minus_one, value_to_extend] = *current_call_frame.stack.pop()?;

        if byte_size_minus_one > U256::from(31) {
            current_call_frame.stack.push(value_to_extend)?;
            return Ok(OpcodeResult::Continue);
        }

        #[allow(
            clippy::arithmetic_side_effects,
            reason = "Since byte_size_minus_one ≤ 31, overflow is impossible"
        )]
        let sign_bit_index = byte_size_minus_one.wrapping_mul(U256::from(8)).wrapping_add(U256::from(7));

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "sign_bit_index max value is 31 * 8 + 7 = 255, which can't overflow."
        )]
        {
            let sign_bit = (value_to_extend >> sign_bit_index) & U256::from(1);
            let mask = (U256::from(1) << sign_bit_index) - U256::from(1);

            let result = if sign_bit.is_zero() {
                value_to_extend & mask
            } else {
                value_to_extend | !mask
            };

            current_call_frame.stack.push(result)?;

            Ok(OpcodeResult::Continue)
        }
    }

    pub fn op_clz(&mut self) -> Result<OpcodeResult, VMError> {
        self.current_call_frame
            .increase_consumed_gas(gas_cost::CLZ)?;

        let value = self.current_call_frame.stack.pop1()?;

        self.current_call_frame
            .stack
            .push(U256::from(value.leading_zeros()))?;

        Ok(OpcodeResult::Continue)
    }
}

/// Shifts the value to the right by 255 bits and checks the most significant bit is a 1
fn is_negative(value: U256) -> bool {
    value.bit(255)
}

/// Negates a number in two's complement
fn negate(value: U256) -> U256 {
    let (dividend, _overflowed) = (!value).overflowing_add(U256::from(1));
    dividend
}

fn abs(value: U256) -> U256 {
    if is_negative(value) {
        negate(value)
    } else {
        value
    }
}
