use crate::{
    call_frame::CallFrame,
    constants::{WORD_SIZE, WORD_SIZE_IN_BYTES_U64},
    errors::{InternalError, OutOfGasError, PrecompileError, VMError},
    memory, StorageSlot,
};
use bytes::Bytes;
/// Contains the gas costs of the EVM instructions
use ethrex_core::U256;
use num_bigint::BigUint;

// Opcodes cost
pub const STOP: u64 = 0;
pub const ADD: u64 = 3;
pub const MUL: u64 = 5;
pub const SUB: u64 = 3;
pub const DIV: u64 = 5;
pub const SDIV: u64 = 5;
pub const MOD: u64 = 5;
pub const SMOD: u64 = 5;
pub const ADDMOD: u64 = 8;
pub const MULMOD: u64 = 8;
pub const EXP_STATIC: u64 = 10;
pub const EXP_DYNAMIC_BASE: u64 = 50;
pub const SIGNEXTEND: u64 = 5;
pub const LT: u64 = 3;
pub const GT: u64 = 3;
pub const SLT: u64 = 3;
pub const SGT: u64 = 3;
pub const EQ: u64 = 3;
pub const ISZERO: u64 = 3;
pub const AND: u64 = 3;
pub const OR: u64 = 3;
pub const XOR: u64 = 3;
pub const NOT: u64 = 3;
pub const BYTE: u64 = 3;
pub const SHL: u64 = 3;
pub const SHR: u64 = 3;
pub const SAR: u64 = 3;
pub const KECCAK25_STATIC: u64 = 30;
pub const KECCAK25_DYNAMIC_BASE: u64 = 6;
pub const CALLDATALOAD: u64 = 3;
pub const CALLDATASIZE: u64 = 2;
pub const CALLDATACOPY_STATIC: u64 = 3;
pub const CALLDATACOPY_DYNAMIC_BASE: u64 = 3;
pub const RETURNDATASIZE: u64 = 2;
pub const RETURNDATACOPY_STATIC: u64 = 3;
pub const RETURNDATACOPY_DYNAMIC_BASE: u64 = 3;
pub const ADDRESS: u64 = 2;
pub const ORIGIN: u64 = 2;
pub const CALLER: u64 = 2;
pub const BLOCKHASH: u64 = 20;
pub const COINBASE: u64 = 2;
pub const TIMESTAMP: u64 = 2;
pub const NUMBER: u64 = 2;
pub const PREVRANDAO: u64 = 2;
pub const GASLIMIT: u64 = 2;
pub const CHAINID: u64 = 2;
pub const SELFBALANCE: u64 = 5;
pub const BASEFEE: u64 = 2;
pub const BLOBHASH: u64 = 3;
pub const BLOBBASEFEE: u64 = 2;
pub const POP: u64 = 2;
pub const MLOAD_STATIC: u64 = 3;
pub const MSTORE_STATIC: u64 = 3;
pub const MSTORE8_STATIC: u64 = 3;
pub const JUMP: u64 = 8;
pub const JUMPI: u64 = 10;
pub const PC: u64 = 2;
pub const MSIZE: u64 = 2;
pub const GAS: u64 = 2;
pub const JUMPDEST: u64 = 1;
pub const TLOAD: u64 = 100;
pub const TSTORE: u64 = 100;
pub const MCOPY_STATIC: u64 = 3;
pub const MCOPY_DYNAMIC_BASE: u64 = 3;
pub const PUSH0: u64 = 2;
pub const PUSHN: u64 = 3;
pub const DUPN: u64 = 3;
pub const SWAPN: u64 = 3;
pub const LOGN_STATIC: u64 = 375;
pub const LOGN_DYNAMIC_BASE: u64 = 375;
pub const LOGN_DYNAMIC_BYTE_BASE: u64 = 8;
pub const CALLVALUE: u64 = 2;
pub const CODESIZE: u64 = 2;
pub const CODECOPY_STATIC: u64 = 3;
pub const CODECOPY_DYNAMIC_BASE: u64 = 3;
pub const GASPRICE: u64 = 2;
pub const SELFDESTRUCT_STATIC: u64 = 5000;
pub const SELFDESTRUCT_DYNAMIC: u64 = 25000;

pub const DEFAULT_STATIC: u64 = 0;
pub const DEFAULT_COLD_DYNAMIC: u64 = 2600;
pub const DEFAULT_WARM_DYNAMIC: u64 = 100;

pub const SLOAD_STATIC: u64 = 0;
pub const SLOAD_COLD_DYNAMIC: u64 = 2100;
pub const SLOAD_WARM_DYNAMIC: u64 = 100;

pub const SSTORE_STATIC: u64 = 0;
pub const SSTORE_COLD_DYNAMIC: u64 = 2100;
pub const SSTORE_DEFAULT_DYNAMIC: u64 = 100;
pub const SSTORE_STORAGE_CREATION: u64 = 20000;
pub const SSTORE_STORAGE_MODIFICATION: u64 = 2900;
pub const SSTORE_STIPEND: u64 = 2300;

pub const BALANCE_STATIC: u64 = DEFAULT_STATIC;
pub const BALANCE_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const BALANCE_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;

pub const EXTCODESIZE_STATIC: u64 = DEFAULT_STATIC;
pub const EXTCODESIZE_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const EXTCODESIZE_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;

pub const EXTCODEHASH_STATIC: u64 = DEFAULT_STATIC;
pub const EXTCODEHASH_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const EXTCODEHASH_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;

pub const EXTCODECOPY_STATIC: u64 = 0;
pub const EXTCODECOPY_DYNAMIC_BASE: u64 = 3;
pub const EXTCODECOPY_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const EXTCODECOPY_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;

pub const CALL_STATIC: u64 = DEFAULT_STATIC;
pub const CALL_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const CALL_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;
pub const CALL_POSITIVE_VALUE: u64 = 9000;
pub const CALL_POSITIVE_VALUE_STIPEND: u64 = 2300;
pub const CALL_TO_EMPTY_ACCOUNT: u64 = 25000;

pub const CALLCODE_STATIC: u64 = DEFAULT_STATIC;
pub const CALLCODE_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const CALLCODE_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;
pub const CALLCODE_POSITIVE_VALUE: u64 = 9000;
pub const CALLCODE_POSITIVE_VALUE_STIPEND: u64 = 2300;

pub const DELEGATECALL_STATIC: u64 = DEFAULT_STATIC;
pub const DELEGATECALL_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const DELEGATECALL_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;

pub const STATICCALL_STATIC: u64 = DEFAULT_STATIC;
pub const STATICCALL_COLD_DYNAMIC: u64 = DEFAULT_COLD_DYNAMIC;
pub const STATICCALL_WARM_DYNAMIC: u64 = DEFAULT_WARM_DYNAMIC;

// Costs in gas for call opcodes
pub const WARM_ADDRESS_ACCESS_COST: u64 = 100;
pub const COLD_ADDRESS_ACCESS_COST: u64 = 2600;
pub const NON_ZERO_VALUE_COST: u64 = 9000;
pub const BASIC_FALLBACK_FUNCTION_STIPEND: u64 = 2300;
pub const VALUE_TO_EMPTY_ACCOUNT_COST: u64 = 25000;

// Costs in gas for create opcodes
pub const INIT_CODE_WORD_COST: u64 = 2;
pub const CODE_DEPOSIT_COST: u64 = 200;
pub const CREATE_BASE_COST: u64 = 32000;

// Calldata costs
pub const CALLDATA_COST_ZERO_BYTE: u64 = 4;
pub const CALLDATA_COST_NON_ZERO_BYTE: u64 = 16;

// Blob gas costs
pub const BLOB_GAS_PER_BLOB: u64 = 131072;

// Access lists costs
pub const ACCESS_LIST_STORAGE_KEY_COST: u64 = 1900;
pub const ACCESS_LIST_ADDRESS_COST: u64 = 2400;

// Precompile costs
pub const ECRECOVER_COST: u64 = 3000;

pub const SHA2_256_STATIC_COST: u64 = 60;
pub const SHA2_256_DYNAMIC_BASE: u64 = 12;

pub const RIPEMD_160_STATIC_COST: u64 = 600;
pub const RIPEMD_160_DYNAMIC_BASE: u64 = 120;

pub const IDENTITY_STATIC_COST: u64 = 15;
pub const IDENTITY_DYNAMIC_BASE: u64 = 3;

pub const MODEXP_STATIC_COST: u64 = 200;
pub const MODEXP_DYNAMIC_QUOTIENT: u64 = 3;

pub fn exp(exponent: U256) -> Result<u64, VMError> {
    let exponent_byte_size = (exponent
        .bits()
        .checked_add(7)
        .ok_or(VMError::OutOfGas(OutOfGasError::GasCostOverflow))?)
        / 8;

    let exponent_byte_size: u64 = exponent_byte_size
        .try_into()
        .map_err(|_| VMError::VeryLargeNumber)?;

    let exponent_byte_size_cost = EXP_DYNAMIC_BASE
        .checked_mul(exponent_byte_size)
        .ok_or(VMError::OutOfGas(OutOfGasError::GasCostOverflow))?;

    EXP_STATIC
        .checked_add(exponent_byte_size_cost)
        .ok_or(VMError::OutOfGas(OutOfGasError::GasCostOverflow))
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
        CALLDATACOPY_STATIC,
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
        CODECOPY_STATIC,
    )
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
        RETURNDATACOPY_STATIC,
    )
}

fn copy_behavior(
    new_memory_size: usize,
    current_memory_size: usize,
    size: usize,
    dynamic_base: u64,
    static_cost: u64,
) -> Result<u64, VMError> {
    let minimum_word_size = (size
        .checked_add(WORD_SIZE)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .saturating_sub(1))
        / WORD_SIZE;

    let minimum_word_size: u64 = minimum_word_size
        .try_into()
        .map_err(|_| VMError::VeryLargeNumber)?;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;
    let memory_expansion_cost: u64 = memory_expansion_cost
        .try_into()
        .map_err(|_| VMError::VeryLargeNumber)?;

    let minimum_word_size_cost = dynamic_base
        .checked_mul(minimum_word_size)
        .ok_or(OutOfGasError::GasCostOverflow)?;
    Ok(static_cost
        .checked_add(minimum_word_size_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .checked_add(memory_expansion_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?)
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
        KECCAK25_STATIC,
    )
}

pub fn log(
    new_memory_size: usize,
    current_memory_size: usize,
    size: usize,
    number_of_topics: u8,
) -> Result<u64, VMError> {
    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;

    let topics_cost = LOGN_DYNAMIC_BASE
        .checked_mul(number_of_topics.into())
        .ok_or(OutOfGasError::GasCostOverflow)?;

    let size: u64 = size.try_into().map_err(|_| VMError::VeryLargeNumber)?;
    let bytes_cost = LOGN_DYNAMIC_BYTE_BASE
        .checked_mul(size)
        .ok_or(OutOfGasError::GasCostOverflow)?;

    let memory_expansion_cost: u64 = memory_expansion_cost
        .try_into()
        .map_err(|_| VMError::VeryLargeNumber)?;

    Ok(topics_cost
        .checked_add(LOGN_STATIC)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .checked_add(bytes_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .checked_add(memory_expansion_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn mload(new_memory_size: usize, current_memory_size: usize) -> Result<u64, VMError> {
    mem_expansion_behavior(new_memory_size, current_memory_size, MLOAD_STATIC)
}

pub fn mstore(new_memory_size: usize, current_memory_size: usize) -> Result<u64, VMError> {
    mem_expansion_behavior(new_memory_size, current_memory_size, MSTORE_STATIC)
}

pub fn mstore8(new_memory_size: usize, current_memory_size: usize) -> Result<u64, VMError> {
    mem_expansion_behavior(new_memory_size, current_memory_size, MSTORE8_STATIC)
}

fn mem_expansion_behavior(
    new_memory_size: usize,
    current_memory_size: usize,
    static_cost: u64,
) -> Result<u64, VMError> {
    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;
    let memory_expansion_cost: u64 = memory_expansion_cost
        .try_into()
        .map_err(|_| VMError::RevertOpcode)?;

    Ok(static_cost
        .checked_add(memory_expansion_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn sload(storage_slot_was_cold: bool) -> Result<u64, VMError> {
    let static_gas = SLOAD_STATIC;

    let dynamic_cost = if storage_slot_was_cold {
        SLOAD_COLD_DYNAMIC
    } else {
        SLOAD_WARM_DYNAMIC
    };

    Ok(static_gas
        .checked_add(dynamic_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn sstore(
    storage_slot: &StorageSlot,
    new_value: U256,
    storage_slot_was_cold: bool,
    current_call_frame: &CallFrame,
) -> Result<u64, VMError> {
    // EIP-2200
    let gas_left = current_call_frame
        .gas_limit
        .checked_sub(current_call_frame.gas_used)
        .ok_or(OutOfGasError::ConsumedGasOverflow)?;
    if gas_left <= SSTORE_STIPEND {
        return Err(VMError::OutOfGas(OutOfGasError::MaxGasLimitExceeded));
    }

    let static_gas = SSTORE_STATIC;

    let mut base_dynamic_gas = if new_value == storage_slot.current_value {
        SSTORE_DEFAULT_DYNAMIC
    } else if storage_slot.current_value == storage_slot.original_value {
        if storage_slot.original_value.is_zero() {
            SSTORE_STORAGE_CREATION
        } else {
            SSTORE_STORAGE_MODIFICATION
        }
    } else {
        SSTORE_DEFAULT_DYNAMIC
    };

    if storage_slot_was_cold {
        base_dynamic_gas = base_dynamic_gas
            .checked_add(SSTORE_COLD_DYNAMIC)
            .ok_or(OutOfGasError::GasCostOverflow)?;
    }

    Ok(static_gas
        .checked_add(base_dynamic_gas)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn mcopy(
    new_memory_size: usize,
    current_memory_size: usize,
    size: usize,
) -> Result<u64, VMError> {
    let words_copied = (size
        .checked_add(WORD_SIZE)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .saturating_sub(1))
        / WORD_SIZE;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;
    let memory_expansion_cost: u64 = memory_expansion_cost
        .try_into()
        .map_err(|_| VMError::VeryLargeNumber)?;

    let words_copied: u64 = words_copied
        .try_into()
        .map_err(|_| VMError::VeryLargeNumber)?;

    let copied_words_cost = MCOPY_DYNAMIC_BASE
        .checked_mul(words_copied)
        .ok_or(OutOfGasError::GasCostOverflow)?;

    Ok(MCOPY_STATIC
        .checked_add(copied_words_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .checked_add(memory_expansion_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn create(
    new_memory_size: usize,
    current_memory_size: usize,
    code_size_in_memory: usize,
) -> Result<u64, VMError> {
    compute_gas_create(
        new_memory_size,
        current_memory_size,
        code_size_in_memory,
        false,
    )
}

pub fn create_2(
    new_memory_size: usize,
    current_memory_size: usize,
    code_size_in_memory: usize,
) -> Result<u64, VMError> {
    compute_gas_create(
        new_memory_size,
        current_memory_size,
        code_size_in_memory,
        true,
    )
}

fn compute_gas_create(
    new_memory_size: usize,
    current_memory_size: usize,
    code_size_in_memory: usize,
    is_create_2: bool,
) -> Result<u64, VMError> {
    let minimum_word_size = (code_size_in_memory
        .checked_add(31)
        .ok_or(OutOfGasError::GasCostOverflow)?)
    .checked_div(32)
    .ok_or(OutOfGasError::ArithmeticOperationDividedByZero)?; // '32' will never be zero

    let minimum_word_size: u64 = minimum_word_size
        .try_into()
        .map_err(|_| VMError::VeryLargeNumber)?;

    let init_code_cost = minimum_word_size
        .checked_mul(INIT_CODE_WORD_COST)
        .ok_or(OutOfGasError::GasCostOverflow)?; // will not panic since it's 2

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;
    let memory_expansion_cost: u64 = memory_expansion_cost
        .try_into()
        .map_err(|_| VMError::VeryLargeNumber)?;

    let hash_cost = if is_create_2 {
        minimum_word_size
            .checked_mul(KECCAK25_DYNAMIC_BASE)
            .ok_or(OutOfGasError::GasCostOverflow)? // will not panic since it's 6
    } else {
        0
    };

    let gas_create_cost = memory_expansion_cost
        .checked_add(init_code_cost)
        .ok_or(OutOfGasError::CreationCostIsTooHigh)?
        .checked_add(CREATE_BASE_COST)
        .ok_or(OutOfGasError::CreationCostIsTooHigh)?
        .checked_add(hash_cost)
        .ok_or(OutOfGasError::CreationCostIsTooHigh)?;

    Ok(gas_create_cost)
}

pub fn selfdestruct(
    address_was_cold: bool,
    account_is_empty: bool,
    balance_to_transfer: U256,
) -> Result<u64, OutOfGasError> {
    let mut gas_cost = SELFDESTRUCT_STATIC;

    if address_was_cold {
        gas_cost = gas_cost
            .checked_add(COLD_ADDRESS_ACCESS_COST)
            .ok_or(OutOfGasError::GasCostOverflow)?;
    }

    // If a positive balance is sent to an empty account, the dynamic gas is 25000
    if account_is_empty && balance_to_transfer > U256::zero() {
        gas_cost = gas_cost
            .checked_add(SELFDESTRUCT_DYNAMIC)
            .ok_or(OutOfGasError::GasCostOverflow)?;
    }

    Ok(gas_cost)
}

pub fn tx_calldata(calldata: &Bytes) -> Result<u64, OutOfGasError> {
    // This cost applies both for call and create
    // 4 gas for each zero byte in the transaction data 16 gas for each non-zero byte in the transaction.
    let mut calldata_cost: u64 = 0;
    for byte in calldata {
        if *byte != 0 {
            calldata_cost = calldata_cost
                .checked_add(CALLDATA_COST_NON_ZERO_BYTE)
                .ok_or(OutOfGasError::GasUsedOverflow)?;
        } else {
            calldata_cost = calldata_cost
                .checked_add(CALLDATA_COST_ZERO_BYTE)
                .ok_or(OutOfGasError::GasUsedOverflow)?;
        }
    }
    Ok(calldata_cost)
}

pub fn tx_creation(code_length: u64, number_of_words: u64) -> Result<u64, OutOfGasError> {
    let mut creation_cost = code_length
        .checked_mul(200)
        .ok_or(OutOfGasError::CreationCostIsTooHigh)?;
    creation_cost = creation_cost
        .checked_add(32000)
        .ok_or(OutOfGasError::CreationCostIsTooHigh)?;

    // GInitCodeword * number_of_words rounded up. GinitCodeWord = 2
    let words_cost = number_of_words
        .checked_mul(2)
        .ok_or(OutOfGasError::GasCostOverflow)?;
    creation_cost
        .checked_add(words_cost)
        .ok_or(OutOfGasError::GasUsedOverflow)
}

fn address_access_cost(
    address_was_cold: bool,
    static_cost: u64,
    cold_dynamic_cost: u64,
    warm_dynamic_cost: u64,
) -> Result<u64, VMError> {
    let static_gas = static_cost;
    let dynamic_cost: u64 = if address_was_cold {
        cold_dynamic_cost
    } else {
        warm_dynamic_cost
    };

    Ok(static_gas
        .checked_add(dynamic_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn balance(address_was_cold: bool) -> Result<u64, VMError> {
    address_access_cost(
        address_was_cold,
        BALANCE_STATIC,
        BALANCE_COLD_DYNAMIC,
        BALANCE_WARM_DYNAMIC,
    )
}

pub fn extcodesize(address_was_cold: bool) -> Result<u64, VMError> {
    address_access_cost(
        address_was_cold,
        EXTCODESIZE_STATIC,
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
        EXTCODECOPY_STATIC,
    )?;
    let expansion_access_cost = address_access_cost(
        address_was_cold,
        EXTCODECOPY_STATIC,
        EXTCODECOPY_COLD_DYNAMIC,
        EXTCODECOPY_WARM_DYNAMIC,
    )?;

    Ok(base_access_cost
        .checked_add(expansion_access_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn extcodehash(address_was_cold: bool) -> Result<u64, VMError> {
    address_access_cost(
        address_was_cold,
        EXTCODEHASH_STATIC,
        EXTCODEHASH_COLD_DYNAMIC,
        EXTCODEHASH_WARM_DYNAMIC,
    )
}

pub fn call(
    new_memory_size: usize,
    current_memory_size: usize,
    address_was_cold: bool,
    address_is_empty: bool,
    value_to_transfer: U256,
) -> Result<u64, VMError> {
    let static_gas = CALL_STATIC;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;
    let memory_expansion_cost: u64 = memory_expansion_cost
        .try_into()
        .map_err(|_| VMError::VeryLargeNumber)?;

    let address_access_cost = address_access_cost(
        address_was_cold,
        CALL_STATIC,
        CALL_COLD_DYNAMIC,
        CALL_WARM_DYNAMIC,
    )?;
    let positive_value_cost = if !value_to_transfer.is_zero() {
        CALL_POSITIVE_VALUE
            .checked_sub(CALL_POSITIVE_VALUE_STIPEND)
            .ok_or(InternalError::ArithmeticOperationUnderflow)?
    } else {
        0
    };
    let value_to_empty_account = if address_is_empty && !value_to_transfer.is_zero() {
        CALL_TO_EMPTY_ACCOUNT
    } else {
        0
    };

    // Note: code_execution_cost will be charged from the sub context post-state.
    let dynamic_gas = memory_expansion_cost
        .checked_add(address_access_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .checked_add(positive_value_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .checked_add(value_to_empty_account)
        .ok_or(OutOfGasError::GasCostOverflow)?;

    Ok(static_gas
        .checked_add(dynamic_gas)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn callcode(
    new_memory_size: usize,
    current_memory_size: usize,
    address_was_cold: bool,
    value_to_transfer: U256,
) -> Result<u64, VMError> {
    let static_gas = CALLCODE_STATIC;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;
    let memory_expansion_cost: u64 = memory_expansion_cost
        .try_into()
        .map_err(|_| VMError::VeryLargeNumber)?;

    let address_access_cost = address_access_cost(
        address_was_cold,
        CALLCODE_STATIC,
        CALLCODE_COLD_DYNAMIC,
        CALLCODE_WARM_DYNAMIC,
    )?;
    let positive_value_cost = if !value_to_transfer.is_zero() {
        CALLCODE_POSITIVE_VALUE
            .checked_sub(CALLCODE_POSITIVE_VALUE_STIPEND)
            .ok_or(InternalError::ArithmeticOperationUnderflow)?
    } else {
        0
    };

    // Note: code_execution_cost will be charged from the sub context post-state.
    let dynamic_gas = memory_expansion_cost
        .checked_add(address_access_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .checked_add(positive_value_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?;

    Ok(static_gas
        .checked_add(dynamic_gas)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn delegatecall(
    new_memory_size: usize,
    current_memory_size: usize,
    address_was_cold: bool,
) -> Result<u64, VMError> {
    let static_gas = DELEGATECALL_STATIC;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;
    let memory_expansion_cost: u64 = memory_expansion_cost
        .try_into()
        .map_err(|_| VMError::VeryLargeNumber)?;

    let address_access_cost = address_access_cost(
        address_was_cold,
        DELEGATECALL_STATIC,
        DELEGATECALL_COLD_DYNAMIC,
        DELEGATECALL_WARM_DYNAMIC,
    )?;

    // Note: code_execution_cost will be charged from the sub context post-state.
    let dynamic_gas = memory_expansion_cost
        .checked_add(address_access_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?;

    Ok(static_gas
        .checked_add(dynamic_gas)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn staticcall(
    new_memory_size: usize,
    current_memory_size: usize,
    address_was_cold: bool,
) -> Result<u64, VMError> {
    let static_gas = STATICCALL_STATIC;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;
    let memory_expansion_cost: u64 = memory_expansion_cost
        .try_into()
        .map_err(|_| VMError::VeryLargeNumber)?;

    let address_access_cost = address_access_cost(
        address_was_cold,
        STATICCALL_STATIC,
        STATICCALL_COLD_DYNAMIC,
        STATICCALL_WARM_DYNAMIC,
    )?;

    // Note: code_execution_cost will be charged from the sub context post-state.
    let dynamic_gas = memory_expansion_cost
        .checked_add(address_access_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?;

    Ok(static_gas
        .checked_add(dynamic_gas)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn fake_exponential(factor: u64, numerator: u64, denominator: u64) -> Result<u64, VMError> {
    let mut i = 1;
    let mut output: u64 = 0;

    // Initial multiplication: factor * denominator
    let mut numerator_accum = factor
        .checked_mul(denominator)
        .ok_or(InternalError::ArithmeticOperationOverflow)?;

    while numerator_accum > 0 {
        // Safe addition to output
        output = output
            .checked_add(numerator_accum)
            .ok_or(InternalError::ArithmeticOperationOverflow)?;

        // Safe multiplication and division within loop
        numerator_accum = numerator_accum
            .checked_mul(numerator)
            .ok_or(InternalError::ArithmeticOperationOverflow)?
            .checked_div(
                denominator
                    .checked_mul(i)
                    .ok_or(InternalError::ArithmeticOperationOverflow)?,
            )
            .ok_or(VMError::Internal(
                InternalError::ArithmeticOperationOverflow,
            ))?;

        i = i
            .checked_add(1)
            .ok_or(InternalError::ArithmeticOperationOverflow)?;
    }

    Ok(output
        .checked_div(denominator)
        .ok_or(InternalError::ArithmeticOperationOverflow)?)
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
    exponent: &BigUint,
    base_size: usize,
    exponent_size: usize,
    modulus_size: usize,
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
    let words = (max_length
        .checked_add(7)
        .ok_or(OutOfGasError::GasCostOverflow)?)
    .checked_div(8)
    .ok_or(InternalError::DivisionError)?;

    let multiplication_complexity = words.checked_pow(2).ok_or(OutOfGasError::GasCostOverflow)?;

    let iteration_count = if exponent_size <= 32 && *exponent != BigUint::ZERO {
        exponent
            .bits()
            .checked_sub(1)
            .ok_or(InternalError::ArithmeticOperationUnderflow)?
    } else if exponent_size > 32 {
        let extra_size = (exponent_size
            .checked_sub(32)
            .ok_or(InternalError::ArithmeticOperationUnderflow)?)
        .checked_mul(8)
        .ok_or(OutOfGasError::GasCostOverflow)?;
        extra_size
            .checked_add(exponent.bits().max(1))
            .ok_or(OutOfGasError::GasCostOverflow)?
            .checked_sub(1)
            .ok_or(InternalError::ArithmeticOperationUnderflow)?
    } else {
        0
    };
    let calculate_iteration_count = iteration_count.max(1);

    let cost = MODEXP_STATIC_COST.max(
        multiplication_complexity
            .checked_mul(calculate_iteration_count)
            .ok_or(OutOfGasError::GasCostOverflow)?
            / MODEXP_DYNAMIC_QUOTIENT,
    );

    Ok(cost)
}

fn precompile(data_size: usize, static_cost: u64, dynamic_base: u64) -> Result<u64, VMError> {
    let data_size: u64 = data_size
        .try_into()
        .map_err(|_| PrecompileError::ParsingInputError)?;

    let data_word_cost = data_size
        .checked_add(WORD_SIZE_IN_BYTES_U64 - 1)
        .ok_or(OutOfGasError::GasCostOverflow)?
        / WORD_SIZE_IN_BYTES_U64;

    let static_gas = static_cost;
    let dynamic_gas = dynamic_base
        .checked_mul(data_word_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?;

    Ok(static_gas
        .checked_add(dynamic_gas)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

/// Max message call gas is all but one 64th of the remaining gas in the current context.
/// https://eips.ethereum.org/EIPS/eip-150
pub fn max_message_call_gas(current_call_frame: &CallFrame) -> Result<u64, VMError> {
    let mut remaining_gas = current_call_frame
        .gas_limit
        .checked_sub(current_call_frame.gas_used)
        .ok_or(InternalError::GasOverflow)?;

    remaining_gas = remaining_gas
        .checked_sub(remaining_gas / 64)
        .ok_or(InternalError::GasOverflow)?;

    Ok(remaining_gas)
}
