#![allow(unexpected_cfgs)]

use crate::{
    errors::{OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};
#[cfg(all(target_vendor = "succinct", target_arch = "riscv32"))]
use crypto_bigint::succinct;
use crypto_bigint::{
    U256 as CryptoU256, Word,
    modular::runtime_mod::{DynResidue, DynResidueParams},
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
            None => U256::zero(),
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

        let product_mod = mulmod_u256(multiplicand, multiplier, modulus);

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

#[inline]
fn to_crypto(value: U256) -> CryptoU256 {
    let mut words = [Word::default(); CryptoU256::LIMBS];

    match CryptoU256::LIMBS {
        // 64-bit host words: one U256 limb maps 1:1
        4 => {
            words
                .iter_mut()
                .zip(value.0)
                .for_each(|(out, limb)| *out = limb as Word);
        }
        // 32-bit host words (SP1): split each 64-bit limb into two 32-bit limbs (little-endian)
        8 => {
            for (i, limb) in value.0.iter().copied().enumerate() {
                let lo = limb as Word;
                let hi = (limb >> 32) as Word;
                let idx = i * 2;
                words[idx] = lo;
                words[idx + 1] = hi;
            }
        }
        _ => unreachable!("unsupported CryptoU256 limb width"),
    }

    CryptoU256::from_words(words)
}

#[inline]
fn from_crypto(value: CryptoU256) -> U256 {
    let words = value.to_words();
    let mut limbs = [0u64; 4];

    match CryptoU256::LIMBS {
        4 => limbs
            .iter_mut()
            .zip(words)
            .for_each(|(out, limb)| *out = limb as u64),
        8 => {
            for i in 0..4 {
                let lo = words[i * 2] as u64;
                let hi = (words[i * 2 + 1] as u64) << 32;
                limbs[i] = lo | hi;
            }
        }
        _ => unreachable!("unsupported CryptoU256 limb width"),
    }

    U256(limbs)
}

fn mulmod_u256(multiplicand: U256, multiplier: U256, modulus: U256) -> U256 {
    if modulus.is_zero() {
        return U256::zero();
    }

    let modulus_crypto = to_crypto(modulus);

    // SP1 path: use the zkVM bigint intrinsic, which is significantly faster than the generic path.
    #[cfg(all(target_vendor = "succinct", target_arch = "riscv32"))]
    {
        let a = to_crypto(multiplicand);
        let b = to_crypto(multiplier);
        let product_mod = succinct::modmul_u256(&a, &b, &modulus_crypto);
        return from_crypto(product_mod);
    }

    if modulus.0[0] & 1 == 0 {
        let product = to_crypto(multiplicand).mul_wide(&to_crypto(multiplier));
        let (remainder, _) = CryptoU256::const_rem_wide(product, &modulus_crypto);

        return from_crypto(remainder);
    }

    let params: DynResidueParams<{ CryptoU256::LIMBS }> = DynResidueParams::new(&modulus_crypto);
    let multiplicand_residue =
        DynResidue::<{ CryptoU256::LIMBS }>::new(&to_crypto(multiplicand), params);
    let multiplier_residue =
        DynResidue::<{ CryptoU256::LIMBS }>::new(&to_crypto(multiplier), params);

    let product = (&multiplicand_residue * &multiplier_residue).retrieve();

    from_crypto(product)
}
