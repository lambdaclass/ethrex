use crate::{
    constants::WORD_SIZE,
    errors::{InternalError, OutOfGasError, VMError},
    memory, StorageSlot,
};
use bytes::Bytes;
/// Contains the gas costs of the EVM instructions (in wei)
use ethrex_core::U256;

// Opcodes cost
pub const ADD: U256 = U256([3, 0, 0, 0]);
pub const MUL: U256 = U256([5, 0, 0, 0]);
pub const SUB: U256 = U256([3, 0, 0, 0]);
pub const DIV: U256 = U256([5, 0, 0, 0]);
pub const SDIV: U256 = U256([5, 0, 0, 0]);
pub const MOD: U256 = U256([5, 0, 0, 0]);
pub const SMOD: U256 = U256([5, 0, 0, 0]);
pub const ADDMOD: U256 = U256([8, 0, 0, 0]);
pub const MULMOD: U256 = U256([8, 0, 0, 0]);
pub const EXP_STATIC: U256 = U256([10, 0, 0, 0]);
pub const EXP_DYNAMIC_BASE: U256 = U256([50, 0, 0, 0]);
pub const SIGNEXTEND: U256 = U256([5, 0, 0, 0]);
pub const LT: U256 = U256([3, 0, 0, 0]);
pub const GT: U256 = U256([3, 0, 0, 0]);
pub const SLT: U256 = U256([3, 0, 0, 0]);
pub const SGT: U256 = U256([3, 0, 0, 0]);
pub const EQ: U256 = U256([3, 0, 0, 0]);
pub const ISZERO: U256 = U256([3, 0, 0, 0]);
pub const AND: U256 = U256([3, 0, 0, 0]);
pub const OR: U256 = U256([3, 0, 0, 0]);
pub const XOR: U256 = U256([3, 0, 0, 0]);
pub const NOT: U256 = U256([3, 0, 0, 0]);
pub const BYTE: U256 = U256([3, 0, 0, 0]);
pub const SHL: U256 = U256([3, 0, 0, 0]);
pub const SHR: U256 = U256([3, 0, 0, 0]);
pub const SAR: U256 = U256([3, 0, 0, 0]);
pub const KECCAK25_STATIC: U256 = U256([30, 0, 0, 0]);
pub const KECCAK25_DYNAMIC_BASE: U256 = U256([6, 0, 0, 0]);
pub const CALLDATALOAD: U256 = U256([3, 0, 0, 0]);
pub const CALLDATASIZE: U256 = U256([2, 0, 0, 0]);
pub const CALLDATACOPY_STATIC: U256 = U256([3, 0, 0, 0]);
pub const CALLDATACOPY_DYNAMIC_BASE: U256 = U256([3, 0, 0, 0]);
pub const RETURNDATASIZE: U256 = U256([2, 0, 0, 0]);
pub const RETURNDATACOPY_STATIC: U256 = U256([3, 0, 0, 0]);
pub const RETURNDATACOPY_DYNAMIC_BASE: U256 = U256([3, 0, 0, 0]);
pub const ADDRESS: U256 = U256([2, 0, 0, 0]);
pub const ORIGIN: U256 = U256([2, 0, 0, 0]);
pub const CALLER: U256 = U256([2, 0, 0, 0]);
pub const BLOCKHASH: U256 = U256([20, 0, 0, 0]);
pub const COINBASE: U256 = U256([2, 0, 0, 0]);
pub const TIMESTAMP: U256 = U256([2, 0, 0, 0]);
pub const NUMBER: U256 = U256([2, 0, 0, 0]);
pub const PREVRANDAO: U256 = U256([2, 0, 0, 0]);
pub const GASLIMIT: U256 = U256([2, 0, 0, 0]);
pub const CHAINID: U256 = U256([2, 0, 0, 0]);
pub const SELFBALANCE: U256 = U256([5, 0, 0, 0]);
pub const BASEFEE: U256 = U256([2, 0, 0, 0]);
pub const BLOBHASH: U256 = U256([3, 0, 0, 0]);
pub const BLOBBASEFEE: U256 = U256([2, 0, 0, 0]);
pub const POP: U256 = U256([2, 0, 0, 0]);
pub const MLOAD_STATIC: U256 = U256([3, 0, 0, 0]);
pub const MSTORE_STATIC: U256 = U256([3, 0, 0, 0]);
pub const MSTORE8_STATIC: U256 = U256([3, 0, 0, 0]);
pub const JUMP: U256 = U256([8, 0, 0, 0]);
pub const JUMPI: U256 = U256([10, 0, 0, 0]);
pub const PC: U256 = U256([2, 0, 0, 0]);
pub const MSIZE: U256 = U256([2, 0, 0, 0]);
pub const GAS: U256 = U256([2, 0, 0, 0]);
pub const JUMPDEST: U256 = U256([1, 0, 0, 0]);
pub const TLOAD: U256 = U256([100, 0, 0, 0]);
pub const TSTORE: U256 = U256([100, 0, 0, 0]);
pub const MCOPY_STATIC: U256 = U256([3, 0, 0, 0]);
pub const MCOPY_DYNAMIC_BASE: U256 = U256([3, 0, 0, 0]);
pub const PUSH0: U256 = U256([2, 0, 0, 0]);
pub const PUSHN: U256 = U256([3, 0, 0, 0]);
pub const DUPN: U256 = U256([3, 0, 0, 0]);
pub const SWAPN: U256 = U256([3, 0, 0, 0]);
pub const LOGN_STATIC: U256 = U256([375, 0, 0, 0]);
pub const LOGN_DYNAMIC_BASE: U256 = U256([375, 0, 0, 0]);
pub const LOGN_DYNAMIC_BYTE_BASE: U256 = U256([8, 0, 0, 0]);
pub const CALLVALUE: U256 = U256([2, 0, 0, 0]);
pub const CODESIZE: U256 = U256([2, 0, 0, 0]);
pub const CODECOPY_STATIC: U256 = U256([3, 0, 0, 0]);
pub const CODECOPY_DYNAMIC_BASE: U256 = U256([3, 0, 0, 0]);
pub const GASPRICE: U256 = U256([2, 0, 0, 0]);
pub const SELFDESTRUCT_STATIC: U256 = U256([5000, 0, 0, 0]);
pub const SELFDESTRUCT_DYNAMIC: U256 = U256([25000, 0, 0, 0]);

pub const DEFAULT_STATIC: U256 = U256::zero();
pub const DEFAULT_COLD_DYNAMIC: U256 = U256([2600, 0, 0, 0]);
pub const DEFAULT_WARM_DYNAMIC: U256 = U256([100, 0, 0, 0]);

pub const SLOAD_STATIC: U256 = U256::zero();
pub const SLOAD_COLD_DYNAMIC: U256 = U256([2100, 0, 0, 0]);
pub const SLOAD_WARM_DYNAMIC: U256 = U256([100, 0, 0, 0]);

pub const SSTORE_STATIC: U256 = U256::zero();
pub const SSTORE_COLD_DYNAMIC: U256 = U256([2100, 0, 0, 0]);
pub const SSTORE_DEFAULT_DYNAMIC: U256 = U256([100, 0, 0, 0]);
pub const SSTORE_STORAGE_CREATION: U256 = U256([20000, 0, 0, 0]);
pub const SSTORE_STORAGE_MODIFICATION: U256 = U256([2900, 0, 0, 0]);

pub const BALANCE_STATIC: U256 = DEFAULT_STATIC;
pub const BALANCE_COLD_DYNAMIC: U256 = DEFAULT_COLD_DYNAMIC;
pub const BALANCE_WARM_DYNAMIC: U256 = DEFAULT_WARM_DYNAMIC;

pub const EXTCODESIZE_STATIC: U256 = DEFAULT_STATIC;
pub const EXTCODESIZE_COLD_DYNAMIC: U256 = DEFAULT_COLD_DYNAMIC;
pub const EXTCODESIZE_WARM_DYNAMIC: U256 = DEFAULT_WARM_DYNAMIC;

pub const EXTCODEHASH_STATIC: U256 = DEFAULT_STATIC;
pub const EXTCODEHASH_COLD_DYNAMIC: U256 = DEFAULT_COLD_DYNAMIC;
pub const EXTCODEHASH_WARM_DYNAMIC: U256 = DEFAULT_WARM_DYNAMIC;

pub const EXTCODECOPY_STATIC: U256 = U256::zero();
pub const EXTCODECOPY_DYNAMIC_BASE: U256 = U256([3, 0, 0, 0]);
pub const EXTCODECOPY_COLD_DYNAMIC: U256 = DEFAULT_COLD_DYNAMIC;
pub const EXTCODECOPY_WARM_DYNAMIC: U256 = DEFAULT_WARM_DYNAMIC;

pub const CALL_STATIC: U256 = DEFAULT_STATIC;
pub const CALL_COLD_DYNAMIC: U256 = DEFAULT_COLD_DYNAMIC;
pub const CALL_WARM_DYNAMIC: U256 = DEFAULT_WARM_DYNAMIC;
pub const CALL_POSITIVE_VALUE: U256 = U256([9000, 0, 0, 0]);
pub const CALL_POSITIVE_VALUE_STIPEND: U256 = U256([2300, 0, 0, 0]);
pub const CALL_TO_EMPTY_ACCOUNT: U256 = U256([25000, 0, 0, 0]);

pub const CALLCODE_STATIC: U256 = DEFAULT_STATIC;
pub const CALLCODE_COLD_DYNAMIC: U256 = DEFAULT_COLD_DYNAMIC;
pub const CALLCODE_WARM_DYNAMIC: U256 = DEFAULT_WARM_DYNAMIC;
pub const CALLCODE_POSITIVE_VALUE: U256 = U256([9000, 0, 0, 0]);
pub const CALLCODE_POSITIVE_VALUE_STIPEND: U256 = U256([2300, 0, 0, 0]);

pub const DELEGATECALL_STATIC: U256 = DEFAULT_STATIC;
pub const DELEGATECALL_COLD_DYNAMIC: U256 = DEFAULT_COLD_DYNAMIC;
pub const DELEGATECALL_WARM_DYNAMIC: U256 = DEFAULT_WARM_DYNAMIC;

pub const STATICCALL_STATIC: U256 = DEFAULT_STATIC;
pub const STATICCALL_COLD_DYNAMIC: U256 = DEFAULT_COLD_DYNAMIC;
pub const STATICCALL_WARM_DYNAMIC: U256 = DEFAULT_WARM_DYNAMIC;

// Costs in gas for call opcodes (in wei)
pub const WARM_ADDRESS_ACCESS_COST: U256 = U256([100, 0, 0, 0]);
pub const COLD_ADDRESS_ACCESS_COST: U256 = U256([2600, 0, 0, 0]);
pub const NON_ZERO_VALUE_COST: U256 = U256([9000, 0, 0, 0]);
pub const BASIC_FALLBACK_FUNCTION_STIPEND: U256 = U256([2300, 0, 0, 0]);
pub const VALUE_TO_EMPTY_ACCOUNT_COST: U256 = U256([25000, 0, 0, 0]);

// Costs in gas for create opcodes (in wei)
pub const INIT_CODE_WORD_COST: U256 = U256([2, 0, 0, 0]);
pub const CODE_DEPOSIT_COST: U256 = U256([200, 0, 0, 0]);
pub const CREATE_BASE_COST: U256 = U256([32000, 0, 0, 0]);

// Calldata costs
pub const CALLDATA_COST_ZERO_BYTE: U256 = U256([4, 0, 0, 0]);
pub const CALLDATA_COST_NON_ZERO_BYTE: U256 = U256([16, 0, 0, 0]);

// Blob gas costs
pub const BLOB_GAS_PER_BLOB: U256 = U256([131072, 0, 0, 0]);

pub fn exp(exponent: U256) -> Result<U256, OutOfGasError> {
    let exponent_byte_size = (exponent
        .bits()
        .checked_add(7)
        .ok_or(OutOfGasError::GasCostOverflow)?)
        / 8;

    let exponent_byte_size_cost = EXP_DYNAMIC_BASE
        .checked_mul(exponent_byte_size.into())
        .ok_or(OutOfGasError::GasCostOverflow)?;

    EXP_STATIC
        .checked_add(exponent_byte_size_cost)
        .ok_or(OutOfGasError::GasCostOverflow)
}

pub fn calldatacopy(
    new_memory_size: usize,
    current_memory_size: usize,
    size: usize,
) -> Result<U256, VMError> {
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
) -> Result<U256, VMError> {
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
) -> Result<U256, VMError> {
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
    dynamic_base: U256,
    static_cost: U256,
) -> Result<U256, VMError> {
    let minimum_word_size = (size
        .checked_add(WORD_SIZE)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .saturating_sub(1))
        / WORD_SIZE;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;

    let minimum_word_size_cost = dynamic_base
        .checked_mul(minimum_word_size.into())
        .ok_or(OutOfGasError::GasCostOverflow)?;
    Ok(static_cost
        .checked_add(minimum_word_size_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .checked_add(memory_expansion_cost.into())
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn keccak256(
    new_memory_size: usize,
    current_memory_size: usize,
    size: usize,
) -> Result<U256, VMError> {
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
) -> Result<U256, VMError> {
    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;

    let topics_cost = LOGN_DYNAMIC_BASE
        .checked_mul(number_of_topics.into())
        .ok_or(OutOfGasError::GasCostOverflow)?;
    let bytes_cost = LOGN_DYNAMIC_BYTE_BASE
        .checked_mul(size.into())
        .ok_or(OutOfGasError::GasCostOverflow)?;
    Ok(topics_cost
        .checked_add(LOGN_STATIC)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .checked_add(bytes_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .checked_add(memory_expansion_cost.into())
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn mload(new_memory_size: usize, current_memory_size: usize) -> Result<U256, VMError> {
    mem_expansion_behavior(new_memory_size, current_memory_size, MLOAD_STATIC)
}

pub fn mstore(new_memory_size: usize, current_memory_size: usize) -> Result<U256, VMError> {
    mem_expansion_behavior(new_memory_size, current_memory_size, MSTORE_STATIC)
}

pub fn mstore8(new_memory_size: usize, current_memory_size: usize) -> Result<U256, VMError> {
    mem_expansion_behavior(new_memory_size, current_memory_size, MSTORE8_STATIC)
}

fn mem_expansion_behavior(
    new_memory_size: usize,
    current_memory_size: usize,
    static_cost: U256,
) -> Result<U256, VMError> {
    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;
    Ok(static_cost
        .checked_add(memory_expansion_cost.into())
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn sload(storage_slot_was_cold: bool) -> Result<U256, VMError> {
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
) -> Result<U256, VMError> {
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
) -> Result<U256, VMError> {
    let words_copied = (size
        .checked_add(WORD_SIZE)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .saturating_sub(1))
        / WORD_SIZE;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;

    let copied_words_cost = MCOPY_DYNAMIC_BASE
        .checked_mul(words_copied.into())
        .ok_or(OutOfGasError::GasCostOverflow)?;

    Ok(MCOPY_STATIC
        .checked_add(copied_words_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?
        .checked_add(memory_expansion_cost.into())
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn create(
    new_memory_size: usize,
    current_memory_size: usize,
    code_size_in_memory: usize,
) -> Result<U256, VMError> {
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
) -> Result<U256, VMError> {
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
) -> Result<U256, VMError> {
    let minimum_word_size = (code_size_in_memory
        .checked_add(31)
        .ok_or(OutOfGasError::GasCostOverflow)?)
    .checked_div(32)
    .ok_or(OutOfGasError::ArithmeticOperationDividedByZero)?; // '32' will never be zero

    let init_code_cost = minimum_word_size
        .checked_mul(INIT_CODE_WORD_COST.as_usize()) // will not panic since it's 2
        .ok_or(OutOfGasError::GasCostOverflow)?;

    let code_deposit_cost = code_size_in_memory
        .checked_mul(CODE_DEPOSIT_COST.as_usize()) // will not panic since it's 200
        .ok_or(OutOfGasError::GasCostOverflow)?;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;

    let hash_cost = if is_create_2 {
        minimum_word_size
            .checked_mul(KECCAK25_DYNAMIC_BASE.as_usize()) // will not panic since it's 6
            .ok_or(OutOfGasError::GasCostOverflow)?
    } else {
        0
    };

    Ok(U256::from(memory_expansion_cost)
        .checked_add(init_code_cost.into())
        .ok_or(OutOfGasError::CreationCostIsTooHigh)?
        .checked_add(code_deposit_cost.into())
        .ok_or(OutOfGasError::CreationCostIsTooHigh)?
        .checked_add(CREATE_BASE_COST)
        .ok_or(OutOfGasError::CreationCostIsTooHigh)?
        .checked_add(hash_cost.into())
        .ok_or(OutOfGasError::CreationCostIsTooHigh)?)
}

pub fn selfdestruct(address_was_cold: bool, account_is_empty: bool) -> Result<U256, OutOfGasError> {
    let mut gas_cost = SELFDESTRUCT_STATIC;

    if address_was_cold {
        gas_cost = gas_cost
            .checked_add(COLD_ADDRESS_ACCESS_COST)
            .ok_or(OutOfGasError::GasCostOverflow)?;
    }

    if account_is_empty {
        gas_cost = gas_cost
            .checked_add(SELFDESTRUCT_DYNAMIC)
            .ok_or(OutOfGasError::GasCostOverflow)?;
    }

    Ok(gas_cost)
}

pub fn tx_calldata(calldata: &Bytes) -> Result<U256, OutOfGasError> {
    // This cost applies both for call and create
    // 4 gas for each zero byte in the transaction data 16 gas for each non-zero byte in the transaction.
    let mut calldata_cost: U256 = U256::zero();
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
    static_cost: U256,
    cold_dynamic_cost: U256,
    warm_dynamic_cost: U256,
) -> Result<U256, VMError> {
    let static_gas = static_cost;
    let dynamic_cost: U256 = if address_was_cold {
        cold_dynamic_cost
    } else {
        warm_dynamic_cost
    };

    Ok(static_gas
        .checked_add(dynamic_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn balance(address_was_cold: bool) -> Result<U256, VMError> {
    address_access_cost(
        address_was_cold,
        BALANCE_STATIC,
        BALANCE_COLD_DYNAMIC,
        BALANCE_WARM_DYNAMIC,
    )
}

pub fn extcodesize(address_was_cold: bool) -> Result<U256, VMError> {
    address_access_cost(
        address_was_cold,
        EXTCODESIZE_STATIC,
        EXTCODESIZE_COLD_DYNAMIC,
        EXTCODESIZE_WARM_DYNAMIC,
    )
}

pub fn extcodecopy(
    new_memory_size: usize,
    current_memory_size: usize,
    address_was_cold: bool,
) -> Result<U256, VMError> {
    Ok(memory::access_cost(
        new_memory_size,
        current_memory_size,
        EXTCODECOPY_STATIC,
        EXTCODECOPY_DYNAMIC_BASE,
    )?
    .checked_add(address_access_cost(
        address_was_cold,
        EXTCODECOPY_STATIC,
        EXTCODECOPY_COLD_DYNAMIC,
        EXTCODECOPY_WARM_DYNAMIC,
    )?)
    .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn extcodehash(address_was_cold: bool) -> Result<U256, VMError> {
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
) -> Result<U256, VMError> {
    let static_gas = CALL_STATIC;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;
    let address_access_cost = address_access_cost(
        address_was_cold,
        CALL_STATIC,
        CALL_COLD_DYNAMIC,
        CALL_WARM_DYNAMIC,
    )?;
    let positive_value_cost = if !value_to_transfer.is_zero() {
        CALL_POSITIVE_VALUE
            .checked_add(CALL_POSITIVE_VALUE_STIPEND)
            .ok_or(InternalError::ArithmeticOperationOverflow)?
    } else {
        U256::zero()
    };
    let value_to_empty_account = if address_is_empty && !value_to_transfer.is_zero() {
        CALL_TO_EMPTY_ACCOUNT
    } else {
        U256::zero()
    };

    // Note: code_execution_cost will be charged from the sub context post-state.
    let dynamic_gas = U256::from(memory_expansion_cost)
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
) -> Result<U256, VMError> {
    let static_gas = CALLCODE_STATIC;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;
    let address_access_cost = address_access_cost(
        address_was_cold,
        CALLCODE_STATIC,
        CALLCODE_COLD_DYNAMIC,
        CALLCODE_WARM_DYNAMIC,
    )?;
    let positive_value_cost = if !value_to_transfer.is_zero() {
        CALLCODE_POSITIVE_VALUE
            .checked_add(CALLCODE_POSITIVE_VALUE_STIPEND)
            .ok_or(InternalError::ArithmeticOperationOverflow)?
    } else {
        U256::zero()
    };

    // Note: code_execution_cost will be charged from the sub context post-state.
    let dynamic_gas = U256::from(memory_expansion_cost)
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
) -> Result<U256, VMError> {
    let static_gas = DELEGATECALL_STATIC;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;
    let address_access_cost = address_access_cost(
        address_was_cold,
        DELEGATECALL_STATIC,
        DELEGATECALL_COLD_DYNAMIC,
        DELEGATECALL_WARM_DYNAMIC,
    )?;

    // Note: code_execution_cost will be charged from the sub context post-state.
    let dynamic_gas = U256::from(memory_expansion_cost)
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
) -> Result<U256, VMError> {
    let static_gas = STATICCALL_STATIC;

    let memory_expansion_cost = memory::expansion_cost(new_memory_size, current_memory_size)?;
    let address_access_cost = address_access_cost(
        address_was_cold,
        STATICCALL_STATIC,
        STATICCALL_COLD_DYNAMIC,
        STATICCALL_WARM_DYNAMIC,
    )?;

    // Note: code_execution_cost will be charged from the sub context post-state.
    let dynamic_gas = U256::from(memory_expansion_cost)
        .checked_add(address_access_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?;

    Ok(static_gas
        .checked_add(dynamic_gas)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

pub fn fake_exponential(factor: u64, numerator: u64, denominator: u64) -> Result<U256, VMError> {
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

    Ok(U256::from(
        output
            .checked_div(denominator)
            .ok_or(InternalError::ArithmeticOperationOverflow)?,
    ))
}
