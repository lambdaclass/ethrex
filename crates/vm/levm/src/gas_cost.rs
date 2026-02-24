use crate::{
    call_frame::CallFrame,
    constants::{WORD_SIZE, WORD_SIZE_IN_BYTES_U64},
    errors::{ExceptionalHalt, InternalError, PrecompileError, VMError},
    memory,
};
use ExceptionalHalt::OutOfGas;
use bytes::Bytes;
/// Contains the gas costs of the EVM instructions
use ethrex_common::{U256, types::Fork};
use malachite::base::num::logic::traits::*;
use malachite::{Natural, base::num::basic::traits::Zero as _};

pub use ethrex_common::types::gas_costs::*;

pub fn exp(exponent: U256) -> Result<u64, VMError> {
    let exponent_byte_size = (exponent.bits().checked_add(7).ok_or(OutOfGas)?) / 8;

    let exponent_byte_size: u64 = exponent_byte_size
        .try_into()
        .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;

    EXP_DYNAMIC_BASE
        .checked_mul(exponent_byte_size)
        .ok_or(OutOfGas.into())
}

pub fn calldatacopy(
    new_memory_size: usize,
    current_memory_size: usize,
    size: usize,
) -> Result<u64, VMError> {
    copy_behavior(
        new_memory_size,
        current_memory_size,
        size,
        CALLDATACOPY_DYNAMIC_BASE,
    )
}

pub fn codecopy(
    new_memory_size: usize,
    current_memory_size: usize,
    size: usize,
) -> Result<u64, VMError> {
    copy_behavior(
        new_memory_size,
        current_memory_size,
        size,
        CODECOPY_DYNAMIC_BASE,
    )
}

// Used in return and revert opcodes
pub fn exit_opcode(new_memory_size: usize, current_memory_size: usize) -> Result<u64, VMError> {
    memory::expansion_cost(new_memory_size, current_memory_size)
}

pub fn returndatacopy(
    new_memory_size: usize,
    current_memory_size: usize,
    size: usize,
) -> Result<u64, VMError> {
    copy_behavior(
        new_memory_size,
        current_memory_size,
        size,
        RETURNDATACOPY_DYNAMIC_BASE,
    )
}

fn copy_behavior(
    new_memory_size: usize,
    current_memory_size: usize,
    size: usize,
    dynamic_base: u64,
) -> Result<u64, VMError> {
    let minimum_word_size = (size
        .checked_add(WORD_SIZE)
        .ok_or(OutOfGas)?
        .saturating_sub(1))
        / WORD_SIZE;

    let minimum_word_size: u64 = minimum_word_size
        .try_into()
        .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;

    let minimum_word_size_cost = dynamic_base
        .checked_mul(minimum_word_size)
        .ok_or(OutOfGas)?;
    minimum_word_size_cost
        .checked_add(memory_expansion_cost)
        .ok_or(OutOfGas.into())
}

pub fn keccak256(
    new_memory_size: usize,
    current_memory_size: usize,
    size: usize,
) -> Result<u64, VMError> {
    copy_behavior(
        new_memory_size,
        current_memory_size,
        size,
        KECCAK25_DYNAMIC_BASE,
    )
}

pub fn log(
    new_memory_size: usize,
    current_memory_size: usize,
    size: usize,
    number_of_topics: usize,
) -> Result<u64, VMError> {
    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;

    // The following conversion can never fail on systems where `usize` is at most 64 bits, which
    // covers every system in production today.
    #[expect(clippy::as_conversions)]
    let topics_cost = LOGN_DYNAMIC_BASE
        .checked_mul(number_of_topics as u64)
        .ok_or(OutOfGas)?;

    let size: u64 = size
        .try_into()
        .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;
    let bytes_cost = LOGN_DYNAMIC_BYTE_BASE.checked_mul(size).ok_or(OutOfGas)?;

    topics_cost
        .checked_add(bytes_cost)
        .ok_or(OutOfGas)?
        .checked_add(memory_expansion_cost)
        .ok_or(OutOfGas.into())
}

pub fn mload(new_memory_size: usize, current_memory_size: usize) -> Result<u64, VMError> {
    memory::expansion_cost(new_memory_size, current_memory_size)
}

pub fn mstore(new_memory_size: usize, current_memory_size: usize) -> Result<u64, VMError> {
    memory::expansion_cost(new_memory_size, current_memory_size)
}

pub fn mstore8(new_memory_size: usize, current_memory_size: usize) -> Result<u64, VMError> {
    memory::expansion_cost(new_memory_size, current_memory_size)
}

pub fn sload(storage_slot_was_cold: bool) -> Result<u64, VMError> {
    let dynamic_cost = if storage_slot_was_cold {
        SLOAD_COLD_DYNAMIC
    } else {
        SLOAD_WARM_DYNAMIC
    };
    Ok(dynamic_cost)
}

pub fn sstore(
    original_value: U256,
    current_value: U256,
    new_value: U256,
    storage_slot_was_cold: bool,
) -> Result<u64, VMError> {
    let mut base_dynamic_gas = if new_value == current_value {
        SSTORE_DEFAULT_DYNAMIC
    } else if current_value == original_value {
        if original_value.is_zero() {
            SSTORE_STORAGE_CREATION
        } else {
            SSTORE_STORAGE_MODIFICATION
        }
    } else {
        SSTORE_DEFAULT_DYNAMIC
    };
    // https://eips.ethereum.org/EIPS/eip-2929
    if storage_slot_was_cold {
        base_dynamic_gas = base_dynamic_gas
            .checked_add(SSTORE_COLD_DYNAMIC)
            .ok_or(OutOfGas)?;
    }
    Ok(base_dynamic_gas)
}

pub fn mcopy(
    new_memory_size: usize,
    current_memory_size: usize,
    size: usize,
) -> Result<u64, VMError> {
    let words_copied = (size
        .checked_add(WORD_SIZE)
        .ok_or(OutOfGas)?
        .saturating_sub(1))
        / WORD_SIZE;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;

    let words_copied: u64 = words_copied
        .try_into()
        .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;

    let copied_words_cost = MCOPY_DYNAMIC_BASE
        .checked_mul(words_copied)
        .ok_or(OutOfGas)?;

    copied_words_cost
        .checked_add(memory_expansion_cost)
        .ok_or(OutOfGas.into())
}

pub fn create(
    new_memory_size: usize,
    current_memory_size: usize,
    code_size_in_memory: usize,
    fork: Fork,
) -> Result<u64, VMError> {
    compute_gas_create(
        new_memory_size,
        current_memory_size,
        code_size_in_memory,
        false,
        fork,
    )
}

pub fn create_2(
    new_memory_size: usize,
    current_memory_size: usize,
    code_size_in_memory: usize,
    fork: Fork,
) -> Result<u64, VMError> {
    compute_gas_create(
        new_memory_size,
        current_memory_size,
        code_size_in_memory,
        true,
        fork,
    )
}

fn compute_gas_create(
    new_memory_size: usize,
    current_memory_size: usize,
    code_size_in_memory: usize,
    is_create_2: bool,
    fork: Fork,
) -> Result<u64, VMError> {
    let minimum_word_size = (code_size_in_memory.checked_add(31).ok_or(OutOfGas)?) / 32;

    let minimum_word_size: u64 = minimum_word_size
        .try_into()
        .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;

    // [EIP-3860] - Apply extra gas cost of 2 for every 32-byte chunk of initcode
    let init_code_cost = if fork >= Fork::Shanghai {
        minimum_word_size
            .checked_mul(INIT_CODE_WORD_COST)
            .ok_or(OutOfGas)? // will not panic since it's 2
    } else {
        0
    };

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;

    let hash_cost = if is_create_2 {
        minimum_word_size
            .checked_mul(KECCAK25_DYNAMIC_BASE)
            .ok_or(OutOfGas)? // will not panic since it's 6
    } else {
        0
    };

    let gas_create_cost = memory_expansion_cost
        .checked_add(init_code_cost)
        .ok_or(OutOfGas)?
        .checked_add(CREATE_BASE_COST)
        .ok_or(OutOfGas)?
        .checked_add(hash_cost)
        .ok_or(OutOfGas)?;

    Ok(gas_create_cost)
}

/// Base cost of SELFDESTRUCT before evaluating NEW_ACCOUNT.
/// Used for EIP-7928 two-phase gas check: first verify base cost is
/// available (to allow BAL state access), then charge the full cost.
pub fn selfdestruct_base(address_was_cold: bool) -> Result<u64, VMError> {
    let cold_cost = if address_was_cold {
        COLD_ADDRESS_ACCESS_COST
    } else {
        0
    };
    SELFDESTRUCT_STATIC
        .checked_add(cold_cost)
        .ok_or(OutOfGas.into())
}

pub fn selfdestruct(
    address_was_cold: bool,
    account_is_empty: bool,
    balance_to_transfer: U256,
) -> Result<u64, VMError> {
    let mut dynamic_cost = if address_was_cold {
        COLD_ADDRESS_ACCESS_COST
    } else {
        0
    };

    // If a positive balance is sent to an empty account, the dynamic gas is 25000
    if account_is_empty && balance_to_transfer > U256::zero() {
        dynamic_cost = dynamic_cost
            .checked_add(SELFDESTRUCT_DYNAMIC)
            .ok_or(OutOfGas)?;
    }

    SELFDESTRUCT_STATIC
        .checked_add(dynamic_cost)
        .ok_or(OutOfGas.into())
}

pub fn tx_calldata(calldata: &Bytes) -> Result<u64, VMError> {
    // This cost applies both for call and create
    // 4 gas for each zero byte in the transaction data 16 gas for each non-zero byte in the transaction.
    let mut calldata_cost: u64 = 0;
    for byte in calldata {
        calldata_cost = if *byte != 0 {
            calldata_cost
                .checked_add(CALLDATA_COST_NON_ZERO_BYTE)
                .ok_or(OutOfGas)?
        } else {
            calldata_cost
                .checked_add(CALLDATA_COST_ZERO_BYTE)
                .ok_or(OutOfGas)?
        }
    }
    Ok(calldata_cost)
}

fn address_access_cost(
    address_was_cold: bool,
    cold_dynamic_cost: u64,
    warm_dynamic_cost: u64,
) -> Result<u64, VMError> {
    let dynamic_cost: u64 = if address_was_cold {
        cold_dynamic_cost
    } else {
        warm_dynamic_cost
    };

    Ok(dynamic_cost)
}

pub fn balance(address_was_cold: bool) -> Result<u64, VMError> {
    address_access_cost(address_was_cold, BALANCE_COLD_DYNAMIC, BALANCE_WARM_DYNAMIC)
}

pub fn extcodesize(address_was_cold: bool) -> Result<u64, VMError> {
    address_access_cost(
        address_was_cold,
        EXTCODESIZE_COLD_DYNAMIC,
        EXTCODESIZE_WARM_DYNAMIC,
    )
}

pub fn extcodecopy(
    size: usize,
    new_memory_size: usize,
    current_memory_size: usize,
    address_was_cold: bool,
) -> Result<u64, VMError> {
    let base_access_cost = copy_behavior(
        new_memory_size,
        current_memory_size,
        size,
        EXTCODECOPY_DYNAMIC_BASE,
    )?;
    let expansion_access_cost = address_access_cost(
        address_was_cold,
        EXTCODECOPY_COLD_DYNAMIC,
        EXTCODECOPY_WARM_DYNAMIC,
    )?;

    base_access_cost
        .checked_add(expansion_access_cost)
        .ok_or(OutOfGas.into())
}

pub fn extcodehash(address_was_cold: bool) -> Result<u64, VMError> {
    address_access_cost(
        address_was_cold,
        EXTCODEHASH_COLD_DYNAMIC,
        EXTCODEHASH_WARM_DYNAMIC,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn call(
    new_memory_size: usize,
    current_memory_size: usize,
    address_was_cold: bool,
    address_is_empty: bool,
    value_to_transfer: U256,
    gas_from_stack: U256,
    gas_left: u64,
) -> Result<(u64, u64), VMError> {
    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;

    let address_access_cost =
        address_access_cost(address_was_cold, CALL_COLD_DYNAMIC, CALL_WARM_DYNAMIC)?;
    let positive_value_cost = if !value_to_transfer.is_zero() {
        CALL_POSITIVE_VALUE
    } else {
        0
    };

    let value_to_empty_account = if address_is_empty && !value_to_transfer.is_zero() {
        CALL_TO_EMPTY_ACCOUNT
    } else {
        0
    };

    let call_gas_costs = memory_expansion_cost
        .checked_add(address_access_cost)
        .ok_or(OutOfGas)?
        .checked_add(positive_value_cost)
        .ok_or(OutOfGas)?
        .checked_add(value_to_empty_account)
        .ok_or(OutOfGas)?;

    calculate_cost_and_gas_limit_call(
        value_to_transfer.is_zero(),
        gas_from_stack,
        gas_left,
        call_gas_costs,
        CALL_POSITIVE_VALUE_STIPEND,
    )
}

pub fn callcode(
    new_memory_size: usize,
    current_memory_size: usize,
    address_was_cold: bool,
    value_to_transfer: U256,
    gas_from_stack: U256,
    gas_left: u64,
) -> Result<(u64, u64), VMError> {
    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;
    let address_access_cost = address_access_cost(
        address_was_cold,
        DELEGATECALL_COLD_DYNAMIC,
        DELEGATECALL_WARM_DYNAMIC,
    )?;

    let positive_value_cost = if !value_to_transfer.is_zero() {
        CALLCODE_POSITIVE_VALUE
    } else {
        0
    };
    let call_gas_costs = memory_expansion_cost
        .checked_add(address_access_cost)
        .ok_or(OutOfGas)?
        .checked_add(positive_value_cost)
        .ok_or(OutOfGas)?;

    calculate_cost_and_gas_limit_call(
        value_to_transfer.is_zero(),
        gas_from_stack,
        gas_left,
        call_gas_costs,
        CALLCODE_POSITIVE_VALUE_STIPEND,
    )
}

pub fn delegatecall(
    new_memory_size: usize,
    current_memory_size: usize,
    address_was_cold: bool,
    gas_from_stack: U256,
    gas_left: u64,
) -> Result<(u64, u64), VMError> {
    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;

    let address_access_cost = address_access_cost(
        address_was_cold,
        DELEGATECALL_COLD_DYNAMIC,
        DELEGATECALL_WARM_DYNAMIC,
    )?;

    let call_gas_costs = memory_expansion_cost
        .checked_add(address_access_cost)
        .ok_or(OutOfGas)?;

    calculate_cost_and_gas_limit_call(true, gas_from_stack, gas_left, call_gas_costs, 0)
}

pub fn staticcall(
    new_memory_size: usize,
    current_memory_size: usize,
    address_was_cold: bool,
    gas_from_stack: U256,
    gas_left: u64,
) -> Result<(u64, u64), VMError> {
    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;

    let address_access_cost = address_access_cost(
        address_was_cold,
        STATICCALL_COLD_DYNAMIC,
        STATICCALL_WARM_DYNAMIC,
    )?;

    let call_gas_costs = memory_expansion_cost
        .checked_add(address_access_cost)
        .ok_or(OutOfGas)?;

    calculate_cost_and_gas_limit_call(true, gas_from_stack, gas_left, call_gas_costs, 0)
}

pub fn sha2_256(data_size: usize) -> Result<u64, VMError> {
    precompile(data_size, SHA2_256_STATIC_COST, SHA2_256_DYNAMIC_BASE)
}

pub fn ripemd_160(data_size: usize) -> Result<u64, VMError> {
    precompile(data_size, RIPEMD_160_STATIC_COST, RIPEMD_160_DYNAMIC_BASE)
}

pub fn identity(data_size: usize) -> Result<u64, VMError> {
    precompile(data_size, IDENTITY_STATIC_COST, IDENTITY_DYNAMIC_BASE)
}

pub fn modexp(
    exponent_first_32_bytes: &Natural,
    base_size: usize,
    exponent_size: usize,
    modulus_size: usize,
    fork: Fork,
) -> Result<u64, VMError> {
    let base_size: u64 = base_size
        .try_into()
        .map_err(|_| PrecompileError::ParsingInputError)?;
    let exponent_size: u64 = exponent_size
        .try_into()
        .map_err(|_| PrecompileError::ParsingInputError)?;
    let modulus_size: u64 = modulus_size
        .try_into()
        .map_err(|_| PrecompileError::ParsingInputError)?;

    let max_length = base_size.max(modulus_size);

    //https://eips.ethereum.org/EIPS/eip-2565

    let words = (max_length.checked_add(7).ok_or(OutOfGas)?) / 8;

    let multiplication_complexity = if fork >= Fork::Osaka {
        if max_length > 32 {
            2_u64
                .checked_mul(words.checked_pow(2).ok_or(OutOfGas)?)
                .ok_or(OutOfGas)?
        } else {
            16
        }
    } else {
        words.checked_pow(2).ok_or(OutOfGas)?
    };

    let modexp_exponent_factor = if fork >= Fork::Osaka {
        MODEXP_EXPONENT_FACTOR_OSAKA
    } else {
        MODEXP_EXPONENT_FACTOR
    };

    let calculate_iteration_count =
        if exponent_size <= 32 && *exponent_first_32_bytes != Natural::ZERO {
            exponent_first_32_bytes
                .significant_bits()
                .checked_sub(1)
                .ok_or(InternalError::Underflow)?
        } else if exponent_size > 32 {
            let extra_size = (exponent_size
                .checked_sub(32)
                .ok_or(InternalError::Underflow)?)
            .checked_mul(modexp_exponent_factor)
            .ok_or(OutOfGas)?;
            extra_size
                .checked_add(exponent_first_32_bytes.significant_bits().max(1))
                .ok_or(OutOfGas)?
                .checked_sub(1)
                .ok_or(InternalError::Underflow)?
        } else {
            0
        }
        .max(1);

    let modexp_static_cost = if fork >= Fork::Osaka {
        MODEXP_STATIC_COST_OSAKA
    } else {
        MODEXP_STATIC_COST
    };

    let modexp_dynamic_quotient = if fork >= Fork::Osaka {
        MODEXP_DYNAMIC_QUOTIENT_OSAKA
    } else {
        MODEXP_DYNAMIC_QUOTIENT
    };

    let cost = modexp_static_cost.max(
        multiplication_complexity
            .checked_mul(calculate_iteration_count)
            .ok_or(OutOfGas)?
            .checked_div(modexp_dynamic_quotient)
            .ok_or(OutOfGas)?,
    );
    Ok(cost)
}

fn precompile(data_size: usize, static_cost: u64, dynamic_base: u64) -> Result<u64, VMError> {
    let data_size: u64 = data_size
        .try_into()
        .map_err(|_| PrecompileError::ParsingInputError)?;

    let data_word_cost = data_size
        .checked_add(WORD_SIZE_IN_BYTES_U64 - 1)
        .ok_or(OutOfGas)?
        / WORD_SIZE_IN_BYTES_U64;

    let static_gas = static_cost;
    let dynamic_gas = dynamic_base.checked_mul(data_word_cost).ok_or(OutOfGas)?;

    static_gas.checked_add(dynamic_gas).ok_or(OutOfGas.into())
}

pub fn ecpairing(groups_number: usize) -> Result<u64, VMError> {
    let groups_number = u64::try_from(groups_number).map_err(|_| InternalError::TypeConversion)?;

    let groups_cost = groups_number
        .checked_mul(ECPAIRING_GROUP_COST)
        .ok_or(OutOfGas)?;
    groups_cost
        .checked_add(ECPAIRING_BASE_COST)
        .ok_or(OutOfGas.into())
}

/// Max message call gas is all but one 64th of the remaining gas in the current context.
/// https://eips.ethereum.org/EIPS/eip-150
#[expect(clippy::arithmetic_side_effects, reason = "can't overflow")]
#[expect(clippy::as_conversions, reason = "remaining gas conversion")]
pub fn max_message_call_gas(current_call_frame: &CallFrame) -> Result<u64, VMError> {
    let mut remaining_gas = current_call_frame.gas_remaining;

    remaining_gas -= remaining_gas / 64;

    Ok(remaining_gas as u64)
}

fn calculate_cost_and_gas_limit_call(
    value_is_zero: bool,
    gas_from_stack: U256,
    gas_left: u64,
    call_gas_costs: u64,
    stipend: u64,
) -> Result<(u64, u64), VMError> {
    let gas_stipend = if value_is_zero { 0 } else { stipend };
    let gas_left = gas_left.checked_sub(call_gas_costs).ok_or(OutOfGas)?;

    // EIP 150, https://eips.ethereum.org/EIPS/eip-150
    let max_gas_for_call = gas_left.checked_sub(gas_left / 64).ok_or(OutOfGas)?;

    let gas: u64 = gas_from_stack
        .min(max_gas_for_call.into())
        .try_into()
        .map_err(|_err| ExceptionalHalt::OutOfGas)?;

    Ok((
        gas.checked_add(call_gas_costs)
            .ok_or(ExceptionalHalt::OutOfGas)?,
        gas.checked_add(gas_stipend)
            .ok_or(ExceptionalHalt::OutOfGas)?,
    ))
}

pub fn bls12_msm(k: usize, discount_table: &[u64; 128], mul_cost: u64) -> Result<u64, VMError> {
    if k == 0 {
        return Ok(0);
    }

    let discount = if k < discount_table.len() {
        discount_table
            .get(k.checked_sub(1).ok_or(InternalError::Underflow)?)
            .copied()
            .ok_or(InternalError::Slicing)?
    } else {
        discount_table
            .last()
            .copied()
            .ok_or(InternalError::Slicing)?
    };

    let gas_cost = u64::try_from(k)
        .map_err(|_| ExceptionalHalt::VeryLargeNumber)?
        .checked_mul(mul_cost)
        .ok_or(ExceptionalHalt::VeryLargeNumber)?
        .checked_mul(discount)
        .ok_or(ExceptionalHalt::VeryLargeNumber)?
        / BLS12_381_MSM_MULTIPLIER;
    Ok(gas_cost)
}

pub fn bls12_pairing_check(k: usize) -> Result<u64, VMError> {
    let gas_cost = u64::try_from(k)
        .map_err(|_| ExceptionalHalt::VeryLargeNumber)?
        .checked_mul(BLS12_PAIRING_CHECK_MUL_COST)
        .ok_or(InternalError::Overflow)?
        .checked_add(BLS12_PAIRING_CHECK_FIXED_COST)
        .ok_or(InternalError::Overflow)?;
    Ok(gas_cost)
}
