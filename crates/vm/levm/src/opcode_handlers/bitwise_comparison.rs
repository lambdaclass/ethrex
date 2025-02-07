use crate::{
    call_frame::CallFrame,
    constants::WORD_SIZE,
    errors::{InternalError, OpcodeResult, VMError},
    gas_cost,
    vm::VM,
};
use ethrex_core::U256;
use std::collections::HashMap;
use std::sync::LazyLock;

// Comparison and Bitwise Logic Operations (14)
// Opcodes: LT, GT, SLT, SGT, EQ, ISZERO, AND, OR, XOR, NOT, BYTE, SHL, SHR, SAR

static SHL_PRECALC: LazyLock<HashMap<u8, U256>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    // Safe shifts (<=63 bits)
    m.insert(8, U256::from(1u64 << 8)); // byte
    m.insert(9, U256::from(1u64 << 9)); // Gwei
    m.insert(12, U256::from(1u64 << 12)); // Szabo
    m.insert(15, U256::from(1u64 << 15)); // Finney
    m.insert(16, U256::from(1u64 << 16)); // uint16
    m.insert(18, U256::from(1u64 << 18)); // Ether
    m.insert(24, U256::from(1u64 << 24)); // 3 bytes
    m.insert(32, U256::from(1u64 << 32)); // uint32
    m.insert(40, U256::from(1u64 << 40)); // 5 bytes
    m.insert(48, U256::from(1u64 << 48)); // 6 bytes
    m.insert(56, U256::from(1u64 << 56)); // 7 bytes
    m.insert(64, U256::from(2).pow(U256::from(64))); // uint64
    m.insert(128, U256::from(2).pow(U256::from(128))); // uint128
    m.insert(160, U256::from(2).pow(U256::from(160))); // address
    m.insert(248, U256::from(2).pow(U256::from(248))); // storage
    m
});

impl VM {
    // LT operation
    pub fn op_lt(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::LT)?;
        let lho = current_call_frame.stack.pop()?;
        let rho = current_call_frame.stack.pop()?;
        let result = u256_from_bool(lho < rho);
        current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // GT operation
    pub fn op_gt(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::GT)?;
        let lho = current_call_frame.stack.pop()?;
        let rho = current_call_frame.stack.pop()?;
        let result = u256_from_bool(lho > rho);
        current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SLT operation (signed less than)
    pub fn op_slt(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::SLT)?;
        let lho = current_call_frame.stack.pop()?;
        let rho = current_call_frame.stack.pop()?;
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

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SGT operation (signed greater than)
    pub fn op_sgt(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::SGT)?;
        let lho = current_call_frame.stack.pop()?;
        let rho = current_call_frame.stack.pop()?;
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

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // EQ operation (equality check)
    pub fn op_eq(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::EQ)?;
        let lho = current_call_frame.stack.pop()?;
        let rho = current_call_frame.stack.pop()?;
        let result = u256_from_bool(lho == rho);

        current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // ISZERO operation (check if zero)
    pub fn op_iszero(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::ISZERO)?;

        let operand = current_call_frame.stack.pop()?;
        let result = u256_from_bool(operand.is_zero());

        current_call_frame.stack.push(result)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // AND operation
    pub fn op_and(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::AND)?;
        let a = current_call_frame.stack.pop()?;
        let b = current_call_frame.stack.pop()?;
        current_call_frame.stack.push(a & b)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // OR operation
    pub fn op_or(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::OR)?;
        let a = current_call_frame.stack.pop()?;
        let b = current_call_frame.stack.pop()?;
        current_call_frame.stack.push(a | b)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // XOR operation
    pub fn op_xor(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::XOR)?;
        let a = current_call_frame.stack.pop()?;
        let b = current_call_frame.stack.pop()?;
        current_call_frame.stack.push(a ^ b)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // NOT operation
    pub fn op_not(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::NOT)?;
        let a = current_call_frame.stack.pop()?;
        current_call_frame.stack.push(!a)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // BYTE operation
    pub fn op_byte(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::BYTE)?;
        let op1 = current_call_frame.stack.pop()?;
        let op2 = current_call_frame.stack.pop()?;
        let byte_index = match op1.try_into() {
            Ok(byte_index) => byte_index,
            Err(_) => {
                // Index is out of bounds, then push 0
                current_call_frame.stack.push(U256::zero())?;
                return Ok(OpcodeResult::Continue { pc_increment: 1 });
            }
        };

        if byte_index < WORD_SIZE {
            let byte_to_push = WORD_SIZE
                .checked_sub(byte_index)
                .ok_or(VMError::Internal(
                    InternalError::ArithmeticOperationUnderflow,
                ))?
                .checked_sub(1)
                .ok_or(VMError::Internal(
                    InternalError::ArithmeticOperationUnderflow,
                ))?; // Same case as above
            current_call_frame
                .stack
                .push(U256::from(op2.byte(byte_to_push)))?;
        } else {
            current_call_frame.stack.push(U256::zero())?;
        }

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SHL operation (shift left)
    pub fn op_shl(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::SHL)?;
        let shift = current_call_frame.stack.pop()?;
        let value = current_call_frame.stack.pop()?;

        if shift.is_zero() {
            current_call_frame.stack.push(value)?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }
        if value.is_zero() {
            current_call_frame.stack.push(U256::zero())?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        // For 1 << n, we can check if we have a precomputed value, and if not use 2^n directly
        if value == U256::one() {
            let res = if shift >= U256::from(256) {
                // Overflow
                U256::zero()
            } else if let Some(precomputed_val) = shl_get_precomputed_value(shift) {
                // Precomputed value in our table
                precomputed_val
            } else {
                // 1<<n but not precomputed, we can calculate 2^n
                // Safe since shift < 256 and 2^255 is the max possible value which fits in U256
                U256::from(2).pow(shift)
            };
            current_call_frame.stack.push(res)?;
            return Ok(OpcodeResult::Continue { pc_increment: 1 });
        }

        // Normal behaviour for values other than 1
        if shift < U256::from(256) {
            current_call_frame
                .stack
                .push(checked_shift_left(value, shift)?)?;
        } else {
            current_call_frame.stack.push(U256::zero())?;
        }

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SHR operation (shift right)
    pub fn op_shr(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::SHR)?;
        let shift = current_call_frame.stack.pop()?;
        let value = current_call_frame.stack.pop()?;

        if shift < U256::from(256) {
            current_call_frame
                .stack
                .push(checked_shift_right(value, shift)?)?;
        } else {
            current_call_frame.stack.push(U256::zero())?;
        }

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }

    // SAR operation (arithmetic shift right)
    pub fn op_sar(&mut self, current_call_frame: &mut CallFrame) -> Result<OpcodeResult, VMError> {
        current_call_frame.increase_consumed_gas(gas_cost::SAR)?;
        let shift = current_call_frame.stack.pop()?;
        let value = current_call_frame.stack.pop()?;
        let res = if shift < U256::from(256) {
            arithmetic_shift_right(value, shift)?
        } else if value.bit(255) {
            U256::MAX
        } else {
            U256::zero()
        };
        current_call_frame.stack.push(res)?;

        Ok(OpcodeResult::Continue { pc_increment: 1 })
    }
}

fn arithmetic_shift_right(value: U256, shift: U256) -> Result<U256, VMError> {
    if value.bit(255) {
        // if negative fill with 1s
        let shifted = checked_shift_right(value, shift)?;
        let mask = checked_shift_left(
            U256::MAX,
            (U256::from(256))
                .checked_sub(shift)
                .ok_or(VMError::Internal(
                    InternalError::ArithmeticOperationUnderflow,
                ))?, // Note that this is already checked in op_sar
        )?;

        Ok(shifted | mask)
    } else {
        Ok(checked_shift_right(value, shift)?)
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
                    .ok_or(VMError::Internal(
                        InternalError::ArithmeticOperationOverflow,
                    ))?;
                let partial_result = result.checked_sub(only_most_representative_bit_on).ok_or(
                    VMError::Internal(InternalError::ArithmeticOperationUnderflow),
                )?; //Should not happen bc checked_mul overflows
                partial_result
                    .checked_mul(2.into())
                    .ok_or(VMError::Internal(
                        InternalError::ArithmeticOperationOverflow,
                    ))?
            }
        };
        shifts_left = shifts_left
            .checked_sub(U256::one())
            .ok_or(VMError::Internal(
                InternalError::ArithmeticOperationUnderflow,
            ))?; // Should not reach negative values
    }

    Ok(result)
}

// Instead of using unsafe >>, uses checked_div n times, replicating n shifts
pub fn checked_shift_right(value: U256, shift: U256) -> Result<U256, VMError> {
    let mut result = value;
    let mut shifts_left = shift;

    while !shifts_left.is_zero() {
        result = result.checked_div(U256::from(2)).ok_or(VMError::Internal(
            InternalError::ArithmeticOperationDividedByZero,
        ))?; // '2' will never be zero
        shifts_left = shifts_left
            .checked_sub(U256::one())
            .ok_or(VMError::Internal(
                InternalError::ArithmeticOperationUnderflow,
            ))?; // Should not reach negative values
    }

    Ok(result)
}

fn u256_from_bool(value: bool) -> U256 {
    U256::from(u8::from(value))
}

fn shl_get_precomputed_value(shift: U256) -> Option<U256> {
    if let Ok(idx) = u8::try_from(shift.as_u64()) {
        SHL_PRECALC.get(&idx).cloned()
    } else {
        None
    }
}
