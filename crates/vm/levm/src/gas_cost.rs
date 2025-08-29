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
pub const SELFDESTRUCT_REFUND: u64 = 24000;

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
pub const STANDARD_TOKEN_COST: u64 = 4;

// Blob gas costs
pub const BLOB_GAS_PER_BLOB: u64 = 131072;

// Access lists costs
pub const ACCESS_LIST_STORAGE_KEY_COST: u64 = 1900;
pub const ACCESS_LIST_ADDRESS_COST: u64 = 2400;

// Precompile costs
pub const ECRECOVER_COST: u64 = 3000;
pub const BLS12_381_G1ADD_COST: u64 = 375;
pub const BLS12_381_G2ADD_COST: u64 = 600;
pub const BLS12_381_MAP_FP_TO_G1_COST: u64 = 5500;
pub const BLS12_PAIRING_CHECK_MUL_COST: u64 = 32600;
pub const BLS12_PAIRING_CHECK_FIXED_COST: u64 = 37700;
pub const BLS12_381_MAP_FP2_TO_G2_COST: u64 = 23800;

// Floor cost per token, specified in https://eips.ethereum.org/EIPS/eip-7623
pub const TOTAL_COST_FLOOR_PER_TOKEN: u64 = 10;

pub const SHA2_256_STATIC_COST: u64 = 60;
pub const SHA2_256_DYNAMIC_BASE: u64 = 12;

pub const RIPEMD_160_STATIC_COST: u64 = 600;
pub const RIPEMD_160_DYNAMIC_BASE: u64 = 120;

pub const IDENTITY_STATIC_COST: u64 = 15;
pub const IDENTITY_DYNAMIC_BASE: u64 = 3;

pub const MODEXP_STATIC_COST: u64 = 200;
pub const MODEXP_DYNAMIC_QUOTIENT: u64 = 3;
pub const MODEXP_EXPONENT_FACTOR: u64 = 8;

pub const MODEXP_STATIC_COST_OSAKA: u64 = 500;
pub const MODEXP_DYNAMIC_QUOTIENT_OSAKA: u64 = 1;
pub const MODEXP_EXPONENT_FACTOR_OSAKA: u64 = 16;

pub const ECADD_COST: u64 = 150;
pub const ECMUL_COST: u64 = 6000;

pub const ECPAIRING_BASE_COST: u64 = 45000;
pub const ECPAIRING_GROUP_COST: u64 = 34000;

pub const POINT_EVALUATION_COST: u64 = 50000;

pub const BLAKE2F_ROUND_COST: u64 = 1;

pub const BLS12_381_MSM_MULTIPLIER: u64 = 1000;
pub const BLS12_381_G1_K_DISCOUNT: [u64; 128] = [
    1000, 949, 848, 797, 764, 750, 738, 728, 719, 712, 705, 698, 692, 687, 682, 677, 673, 669, 665,
    661, 658, 654, 651, 648, 645, 642, 640, 637, 635, 632, 630, 627, 625, 623, 621, 619, 617, 615,
    613, 611, 609, 608, 606, 604, 603, 601, 599, 598, 596, 595, 593, 592, 591, 589, 588, 586, 585,
    584, 582, 581, 580, 579, 577, 576, 575, 574, 573, 572, 570, 569, 568, 567, 566, 565, 564, 563,
    562, 561, 560, 559, 558, 557, 556, 555, 554, 553, 552, 551, 550, 549, 548, 547, 547, 546, 545,
    544, 543, 542, 541, 540, 540, 539, 538, 537, 536, 536, 535, 534, 533, 532, 532, 531, 530, 529,
    528, 528, 527, 526, 525, 525, 524, 523, 522, 522, 521, 520, 520, 519,
];
pub const G1_MUL_COST: u64 = 12000;
pub const BLS12_381_G2_K_DISCOUNT: [u64; 128] = [
    1000, 1000, 923, 884, 855, 832, 812, 796, 782, 770, 759, 749, 740, 732, 724, 717, 711, 704,
    699, 693, 688, 683, 679, 674, 670, 666, 663, 659, 655, 652, 649, 646, 643, 640, 637, 634, 632,
    629, 627, 624, 622, 620, 618, 615, 613, 611, 609, 607, 606, 604, 602, 600, 598, 597, 595, 593,
    592, 590, 589, 587, 586, 584, 583, 582, 580, 579, 578, 576, 575, 574, 573, 571, 570, 569, 568,
    567, 566, 565, 563, 562, 561, 560, 559, 558, 557, 556, 555, 554, 553, 552, 552, 551, 550, 549,
    548, 547, 546, 545, 545, 544, 543, 542, 541, 541, 540, 539, 538, 537, 537, 536, 535, 535, 534,
    533, 532, 532, 531, 530, 530, 529, 528, 528, 527, 526, 526, 525, 524, 524,
];
pub const G2_MUL_COST: u64 = 22500;

pub fn exp(exponent: U256) -> Result<u64, VMError> {
    let exponent_byte_size = (exponent.bits().checked_add(7).ok_or(OutOfGas)?) / 8;

    let exponent_byte_size: u64 = exponent_byte_size
        .try_into()
        .map_err(|_| ExceptionalHalt::VeryLargeNumber)?;

    let exponent_byte_size_cost = EXP_DYNAMIC_BASE
        .checked_mul(exponent_byte_size)
        .ok_or(OutOfGas)?;

    EXP_STATIC
        .checked_add(exponent_byte_size_cost)
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
    static_cost
        .checked_add(minimum_word_size_cost)
        .ok_or(OutOfGas)?
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
        KECCAK25_STATIC,
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
        .checked_add(LOGN_STATIC)
        .ok_or(OutOfGas)?
        .checked_add(bytes_cost)
        .ok_or(OutOfGas)?
        .checked_add(memory_expansion_cost)
        .ok_or(OutOfGas.into())
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

    static_cost
        .checked_add(memory_expansion_cost)
        .ok_or(OutOfGas.into())
}

pub fn sload(storage_slot_was_cold: bool) -> Result<u64, VMError> {
    let static_gas = SLOAD_STATIC;
    let dynamic_cost = if storage_slot_was_cold {
        SLOAD_COLD_DYNAMIC
    } else {
        SLOAD_WARM_DYNAMIC
    };
    static_gas.checked_add(dynamic_cost).ok_or(OutOfGas.into())
}

pub fn sstore(
    original_value: U256,
    current_value: U256,
    new_value: U256,
    storage_slot_was_cold: bool,
) -> Result<u64, VMError> {
    let static_gas = SSTORE_STATIC;

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
    static_gas
        .checked_add(base_dynamic_gas)
        .ok_or(OutOfGas.into())
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

    MCOPY_STATIC
        .checked_add(copied_words_cost)
        .ok_or(OutOfGas)?
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

pub fn tx_creation(code_length: u64, number_of_words: u64) -> Result<u64, VMError> {
    let mut creation_cost = code_length.checked_mul(CODE_DEPOSIT_COST).ok_or(OutOfGas)?;
    creation_cost = creation_cost
        .checked_add(CREATE_BASE_COST)
        .ok_or(OutOfGas)?;

    // GInitCodeword * number_of_words rounded up. GinitCodeWord = 2
    let words_cost = number_of_words.checked_mul(2).ok_or(OutOfGas)?;
    creation_cost.checked_add(words_cost).ok_or(OutOfGas.into())
}

fn address_access_cost(
    address_was_cold: bool,
    static_cost: u64,
    cold_dynamic_cost: u64,
    warm_dynamic_cost: u64,
) -> Result<u64, VMError> {
    let dynamic_cost: u64 = if address_was_cold {
        cold_dynamic_cost
    } else {
        warm_dynamic_cost
    };

    static_cost.checked_add(dynamic_cost).ok_or(OutOfGas.into())
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

    base_access_cost
        .checked_add(expansion_access_cost)
        .ok_or(OutOfGas.into())
}

pub fn extcodehash(address_was_cold: bool) -> Result<u64, VMError> {
    address_access_cost(
        address_was_cold,
        EXTCODEHASH_STATIC,
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

    let address_access_cost = address_access_cost(
        address_was_cold,
        CALL_STATIC,
        CALL_COLD_DYNAMIC,
        CALL_WARM_DYNAMIC,
    )?;
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
        DELEGATECALL_STATIC,
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
        DELEGATECALL_STATIC,
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
        STATICCALL_STATIC,
        STATICCALL_COLD_DYNAMIC,
        STATICCALL_WARM_DYNAMIC,
    )?;

    let call_gas_costs = memory_expansion_cost
        .checked_add(address_access_cost)
        .ok_or(OutOfGas)?;

    calculate_cost_and_gas_limit_call(true, gas_from_stack, gas_left, call_gas_costs, 0)
}

/// Approximates factor * e ** (numerator / denominator) using Taylor expansion
/// https://eips.ethereum.org/EIPS/eip-4844#helpers
pub fn fake_exponential(factor: U256, numerator: U256, denominator: u64) -> Result<U256, VMError> {
    if denominator == 0 {
        return Err(InternalError::DivisionByZero.into());
    }

    if numerator.is_zero() {
        return Ok(factor);
    }

    let mut output: U256 = U256::zero();
    let denominator_u256: U256 = denominator.into();

    // Initial multiplication: factor * denominator
    let mut numerator_accum = factor
        .checked_mul(denominator_u256)
        .ok_or(InternalError::Overflow)?;

    let mut denominator_by_i = denominator_u256;

    #[expect(
        clippy::arithmetic_side_effects,
        reason = "division can't overflow since denominator is not 0"
    )]
    {
        while !numerator_accum.is_zero() {
            // Safe addition to output
            output = output
                .checked_add(numerator_accum)
                .ok_or(InternalError::Overflow)?;

            // Safe multiplication and division within loop
            numerator_accum = numerator_accum
                .checked_mul(numerator)
                .ok_or(InternalError::Overflow)?
                / denominator_by_i;

            // denominator comes from a u64 value, will never overflow before other variables.
            denominator_by_i += denominator_u256;
        }

        Ok(output / denominator)
    }
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

    let multiplication_complexity = match fork {
        Fork::Osaka => {
            if max_length > 32 {
                2_u64
                    .checked_mul(words.checked_pow(2).ok_or(OutOfGas)?)
                    .ok_or(OutOfGas)?
            } else {
                16
            }
        }
        _ => words.checked_pow(2).ok_or(OutOfGas)?,
    };

    let modexp_exponent_factor = match fork {
        Fork::Osaka => MODEXP_EXPONENT_FACTOR_OSAKA,
        _ => MODEXP_EXPONENT_FACTOR,
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

    let modexp_static_cost = match fork {
        Fork::Osaka => MODEXP_STATIC_COST_OSAKA,
        _ => MODEXP_STATIC_COST,
    };

    let modexp_dynamic_quotient = match fork {
        Fork::Osaka => MODEXP_DYNAMIC_QUOTIENT_OSAKA,
        _ => MODEXP_DYNAMIC_QUOTIENT,
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
pub fn max_message_call_gas(current_call_frame: &CallFrame) -> Result<u64, VMError> {
    let mut remaining_gas = current_call_frame.gas_remaining;

    remaining_gas = remaining_gas
        .checked_sub(remaining_gas / 64)
        .ok_or(InternalError::Underflow)?;

    Ok(remaining_gas)
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
