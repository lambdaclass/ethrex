use crate::{
    U256,
    constants::{TWO_FIFTY_SIX, WORD_SIZE},
    errors::{InternalError, OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};

// Comparison and Bitwise Logic Operations (14)
// Opcodes: LT, GT, SLT, SGT, EQ, ISZERO, AND, OR, XOR, NOT, BYTE, SHL, SHR, SAR

impl<'a> VM<'a> {
    // LT operation
    #[inline]
    pub fn op_lt(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::LT)?;
        let [lho, rho] = *current_call_frame.stack.pop()?;
        let result = u256_from_bool(lho < rho);
        current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }

    // GT operation
    #[inline]
    pub fn op_gt(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::GT)?;
        let [lho, rho] = *current_call_frame.stack.pop()?;
        let result = u256_from_bool(lho > rho);
        current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }

    // SLT operation (signed less than)
    pub fn op_slt(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::SLT)?;
        let [lho, rho] = *current_call_frame.stack.pop()?;
        let lho_is_negative = lho.bit(255);
        let rho_is_negative = rho.bit(255);
        let result = if lho_is_negative == rho_is_negative {
            // Compare magnitudes if signs are the same
            u256_from_bool(lho < rho)
        } else {
            // Negative is smaller if signs differ
            u256_from_bool(lho_is_negative)
        };
        current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }

    // SGT operation (signed greater than)
    pub fn op_sgt(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::SGT)?;
        let [lho, rho] = *current_call_frame.stack.pop()?;
        let lho_is_negative = lho.bit(255);
        let rho_is_negative = rho.bit(255);
        let result = if lho_is_negative == rho_is_negative {
            // Compare magnitudes if signs are the same
            u256_from_bool(lho > rho)
        } else {
            // Positive is bigger if signs differ
            u256_from_bool(rho_is_negative)
        };
        current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }

    // EQ operation (equality check)
    #[inline]
    pub fn op_eq(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::EQ)?;
        let [lho, rho] = *current_call_frame.stack.pop()?;
        let result = u256_from_bool(lho == rho);

        current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }

    // ISZERO operation (check if zero)
    pub fn op_iszero(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::ISZERO)?;

        let [operand] = current_call_frame.stack.pop()?;
        let result = u256_from_bool(operand.is_zero());

        current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue)
    }

    // AND operation
    #[inline]
    pub fn op_and(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::AND)?;
        let [a, b] = *current_call_frame.stack.pop()?;
        current_call_frame.stack.push(a & b)?;

        Ok(OpcodeResult::Continue)
    }

    // OR operation
    #[inline]
    pub fn op_or(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::OR)?;
        let [a, b] = *current_call_frame.stack.pop()?;
        current_call_frame.stack.push(a | b)?;

        Ok(OpcodeResult::Continue)
    }

    // XOR operation
    #[inline]
    pub fn op_xor(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::XOR)?;
        let [a, b] = *current_call_frame.stack.pop()?;
        current_call_frame.stack.push(a ^ b)?;

        Ok(OpcodeResult::Continue)
    }

    // NOT operation
    pub fn op_not(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::NOT)?;
        let a = current_call_frame.stack.pop1()?;
        current_call_frame.stack.push(!a)?;

        Ok(OpcodeResult::Continue)
    }

    // BYTE operation
    pub fn op_byte(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::BYTE)?;
        let [op1, op2] = *current_call_frame.stack.pop()?;
        let byte_index = match op1.try_into() {
            Ok(byte_index) => byte_index,
            Err(_) => {
                // Index is out of bounds, then push 0
                current_call_frame.stack.push_zero()?;
                return Ok(OpcodeResult::Continue);
            }
        };

        if byte_index < WORD_SIZE {
            let byte_to_push = WORD_SIZE
                .checked_sub(byte_index)
                .ok_or(InternalError::Underflow)?
                .checked_sub(1)
                .ok_or(InternalError::Underflow)?; // Same case as above
            current_call_frame
                .stack
                .push(U256::from(op2.byte(byte_to_push)))?;
        } else {
            current_call_frame.stack.push_zero()?;
        }

        Ok(OpcodeResult::Continue)
    }

    #[expect(clippy::arithmetic_side_effects)]
    // SHL operation (shift left)
    #[inline]
    pub fn op_shl(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::SHL)?;
        let [shift, value] = *current_call_frame.stack.pop()?;

        if shift < TWO_FIFTY_SIX {
            current_call_frame.stack.push(value << shift)?;
        } else {
            current_call_frame.stack.push_zero()?;
        }

        Ok(OpcodeResult::Continue)
    }

    #[expect(clippy::arithmetic_side_effects)]
    // SHR operation (shift right)
    #[inline]
    pub fn op_shr(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::SHR)?;
        let [shift, value] = *current_call_frame.stack.pop()?;

        if shift < TWO_FIFTY_SIX {
            current_call_frame.stack.push(value >> shift)?;
        } else {
            current_call_frame.stack.push_zero()?;
        }

        Ok(OpcodeResult::Continue)
    }

    #[allow(clippy::arithmetic_side_effects)]
    // SAR operation (arithmetic shift right)
    #[inline]
    pub fn op_sar(&mut self) -> Result<OpcodeResult, VMError> {
        let current_call_frame = &mut self.current_call_frame;
        current_call_frame.increase_consumed_gas(gas_cost::SAR)?;
        let [shift, value] = *current_call_frame.stack.pop()?;

        // In 2's complement arithmetic, the most significant bit being one means the number is negative
        let is_negative = value.bit(255);

        let res = if shift < TWO_FIFTY_SIX {
            if !is_negative {
                value >> shift
            } else {
                (value >> shift) | ((U256::MAX) << (TWO_FIFTY_SIX - shift))
            }
        } else if is_negative {
            U256::MAX
        } else {
            U256::ZERO
        };
        current_call_frame.stack.push(res)?;

        Ok(OpcodeResult::Continue)
    }
}

const ONE: U256 = U256::from_limbs([1, 0, 0, 0]);

const fn u256_from_bool(value: bool) -> U256 {
    if value { ONE } else { U256::ZERO }
}
