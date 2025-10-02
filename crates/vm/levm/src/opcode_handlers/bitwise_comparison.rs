use std::cell::OnceCell;

use crate::{
    constants::WORD_SIZE,
    errors::{InternalError, OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};
use ethrex_common::U256;

// Comparison and Bitwise Logic Operations (14)
// Opcodes: LT, GT, SLT, SGT, EQ, ISZERO, AND, OR, XOR, NOT, BYTE, SHL, SHR, SAR

impl<'a> VM<'a> {
    // LT operation
    pub fn op_lt(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::LT) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [lho, rho] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let result = u256_from_bool(lho < rho);
        if let Err(err) = self.current_call_frame.stack.push1(result) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // GT operation
    pub fn op_gt(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::GT) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [lho, rho] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let result = u256_from_bool(lho > rho);
        if let Err(err) = self.current_call_frame.stack.push1(result) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // SLT operation (signed less than)
    pub fn op_slt(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::SLT) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [lho, rho] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let lho_is_negative = lho.bit(255);
        let rho_is_negative = rho.bit(255);
        let result = if lho_is_negative == rho_is_negative {
            // Compare magnitudes if signs are the same
            u256_from_bool(lho < rho)
        } else {
            // Negative is smaller if signs differ
            u256_from_bool(lho_is_negative)
        };
        if let Err(err) = self.current_call_frame.stack.push1(result) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // SGT operation (signed greater than)
    pub fn op_sgt(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::SGT) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [lho, rho] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let lho_is_negative = lho.bit(255);
        let rho_is_negative = rho.bit(255);
        let result = if lho_is_negative == rho_is_negative {
            // Compare magnitudes if signs are the same
            u256_from_bool(lho > rho)
        } else {
            // Positive is bigger if signs differ
            u256_from_bool(rho_is_negative)
        };
        if let Err(err) = self.current_call_frame.stack.push1(result) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // EQ operation (equality check)
    pub fn op_eq(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::EQ) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [lho, rho] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let result = u256_from_bool(lho == rho);
        if let Err(err) = self.current_call_frame.stack.push1(result) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // ISZERO operation (check if zero)
    pub fn op_iszero(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::ISZERO)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [operand] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let result = u256_from_bool(operand.is_zero());

        if let Err(err) = self.current_call_frame.stack.push1(result) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // AND operation
    pub fn op_and(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::AND) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [a, b] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        if let Err(err) = self.current_call_frame.stack.push(&[a & b]) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // OR operation
    pub fn op_or(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::OR) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [a, b] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        if let Err(err) = self.current_call_frame.stack.push(&[a | b]) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // XOR operation
    pub fn op_xor(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::XOR) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [a, b] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        if let Err(err) = self.current_call_frame.stack.push(&[a ^ b]) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // NOT operation
    pub fn op_not(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::NOT) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let a = match self.current_call_frame.stack.pop1() {
            Ok(x) => x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        if let Err(err) = self.current_call_frame.stack.push(&[!a]) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }

    // BYTE operation
    pub fn op_byte(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self
            .current_call_frame
            .increase_consumed_gas(gas_cost::BYTE)
        {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [op1, op2] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };
        let byte_index = match op1.try_into() {
            Ok(byte_index) => byte_index,
            Err(_) => {
                // Index is out of bounds, then push 0
                if let Err(err) = self.current_call_frame.stack.push_zero() {
                    error.set(err.into());
                    return OpcodeResult::Halt;
                }
                return OpcodeResult::Continue;
            }
        };

        if byte_index < WORD_SIZE {
            let byte_to_push = match WORD_SIZE
                .checked_sub(byte_index)
                .and_then(|x| x.checked_sub(1))
            {
                Some(x) => x,
                None => {
                    error.set(InternalError::Underflow.into());
                    return OpcodeResult::Halt;
                }
            };
            if let Err(err) = self
                .current_call_frame
                .stack
                .push(&[U256::from(op2.byte(byte_to_push))])
            {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        } else {
            if let Err(err) = self.current_call_frame.stack.push_zero() {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        }

        OpcodeResult::Continue
    }

    #[expect(clippy::arithmetic_side_effects)]
    // SHL operation (shift left)
    pub fn op_shl(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::SHL) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [shift, value] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if shift < U256::from(256) {
            if let Err(err) = self.current_call_frame.stack.push(&[value << shift]) {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        } else {
            if let Err(err) = self.current_call_frame.stack.push_zero() {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        }

        OpcodeResult::Continue
    }

    #[expect(clippy::arithmetic_side_effects)]
    // SHR operation (shift right)
    pub fn op_shr(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::SHR) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [shift, value] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        if shift < U256::from(256) {
            if let Err(err) = self.current_call_frame.stack.push(&[value >> shift]) {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        } else {
            if let Err(err) = self.current_call_frame.stack.push_zero() {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        }

        OpcodeResult::Continue
    }

    #[allow(clippy::arithmetic_side_effects)]
    // SAR operation (arithmetic shift right)
    pub fn op_sar(&mut self, error: &mut OnceCell<VMError>) -> OpcodeResult {
        if let Err(err) = self.current_call_frame.increase_consumed_gas(gas_cost::SAR) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        let [shift, value] = match self.current_call_frame.stack.pop() {
            Ok(x) => *x,
            Err(err) => {
                error.set(err.into());
                return OpcodeResult::Halt;
            }
        };

        // In 2's complement arithmetic, the most significant bit being one means the number is negative
        let is_negative = value.bit(255);

        let res = if shift < U256::from(256) {
            if !is_negative {
                value >> shift
            } else {
                (value >> shift) | ((U256::MAX) << (U256::from(256) - shift))
            }
        } else if is_negative {
            U256::MAX
        } else {
            U256::zero()
        };

        if let Err(err) = self.current_call_frame.stack.push1(res) {
            error.set(err.into());
            return OpcodeResult::Halt;
        }

        OpcodeResult::Continue
    }
}

/// Instead of using unsafe <<, uses checked_mul n times, replicating n shifts.
/// Note: These (checked_shift_left and checked_shift_right) are done because
/// are not available in U256
pub fn checked_shift_left(value: U256, shift: U256) -> Result<U256, VMError> {
    let mut result = value;
    let mut shifts_left = shift;

    while !shifts_left.is_zero() {
        result = match result.checked_mul(U256::from(2)) {
            Some(num) => num,
            None => {
                let only_most_representative_bit_on = U256::from(2)
                    .checked_pow(U256::from(255))
                    .ok_or(InternalError::Overflow)?;
                let partial_result = result
                    .checked_sub(only_most_representative_bit_on)
                    .ok_or(InternalError::Underflow)?; //Should not happen bc checked_mul overflows
                partial_result
                    .checked_mul(2.into())
                    .ok_or(InternalError::Overflow)?
            }
        };
        shifts_left = shifts_left
            .checked_sub(U256::one())
            .ok_or(InternalError::Underflow)?; // Should not reach negative values
    }

    Ok(result)
}

const fn u256_from_bool(value: bool) -> U256 {
    if value { U256::one() } else { U256::zero() }
}
