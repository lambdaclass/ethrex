use crate::errors::{InternalError, VMError};
use ethereum_rust_core::U256;

pub const SUCCESS_FOR_CALL: i32 = 1;
pub const REVERT_FOR_CALL: i32 = 0;
pub const HALT_FOR_CALL: i32 = 2;
pub const SUCCESS_FOR_RETURN: i32 = 1;
pub const REVERT_FOR_CREATE: i32 = 0;
pub const WORD_SIZE: usize = 32;

/// Contains the gas costs of the EVM instructions (in wei)
pub mod gas_cost {
    use ethereum_rust_core::U256;

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
    pub const EXTCODECOPY_DYNAMIC_BASE: U256 = U256([3, 0, 0, 0]);
    pub const SELFDESTRUCT_STATIC: U256 = U256([5000, 0, 0, 0]);
    pub const SELFDESTRUCT_DYNAMIC: U256 = U256([25000, 0, 0, 0]);
    pub const COLD_ADDRESS_ACCESS_COST: U256 = U256([2600, 0, 0, 0]);
}

// Costs in gas for call opcodes (in wei)
pub mod call_opcode {
    use ethereum_rust_core::U256;

    pub const WARM_ADDRESS_ACCESS_COST: U256 = U256([100, 0, 0, 0]);
    pub const COLD_ADDRESS_ACCESS_COST: U256 = U256([2600, 0, 0, 0]);
    pub const NON_ZERO_VALUE_COST: U256 = U256([9000, 0, 0, 0]);
    pub const BASIC_FALLBACK_FUNCTION_STIPEND: U256 = U256([2300, 0, 0, 0]);
    pub const VALUE_TO_EMPTY_ACCOUNT_COST: U256 = U256([25000, 0, 0, 0]);
}
pub const STACK_LIMIT: usize = 1024;

pub const GAS_REFUND_DENOMINATOR: u64 = 5;

pub const EMPTY_CODE_HASH_STR: &str =
    "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470";

pub const MEMORY_EXPANSION_QUOTIENT: usize = 512;

// Transaction costs in gas (in wei)
pub const TX_BASE_COST: U256 = U256([21000, 0, 0, 0]);

pub const MAX_CODE_SIZE: usize = 0x6000;
pub const MAX_CREATE_CODE_SIZE: usize = 2 * MAX_CODE_SIZE;

pub const INVALID_CONTRACT_PREFIX: u8 = 0xef;

// Costs in gas for init word and init code (in wei)
pub const INIT_WORD_COST: u64 = 2;

pub fn init_code_cost(init_code_length: usize) -> Result<u64, VMError> {
    let length_u64 = u64::try_from(init_code_length)
        .map_err(|_| VMError::Internal(InternalError::ConversionError))?;
    Ok(INIT_WORD_COST * (length_u64 + 31) / 32)
}
pub mod create_opcode {
    use ethereum_rust_core::U256;

    pub const INIT_CODE_WORD_COST: U256 = U256([2, 0, 0, 0]);
    pub const CODE_DEPOSIT_COST: U256 = U256([200, 0, 0, 0]);
    pub const CREATE_BASE_COST: U256 = U256([32000, 0, 0, 0]);
}

pub const VERSIONED_HASH_VERSION_KZG: u8 = 0x01;
pub const MAX_BLOB_NUMBER_PER_BLOCK: usize = 6;

// Blob constants
pub const TARGET_BLOB_GAS_PER_BLOCK: U256 = U256([393216, 0, 0, 0]); // TARGET_BLOB_NUMBER_PER_BLOCK * GAS_PER_BLOB
pub const MIN_BASE_FEE_PER_BLOB_GAS: U256 = U256([1, 0, 0, 0]);
pub const BLOB_BASE_FEE_UPDATE_FRACTION: U256 = U256([3338477, 0, 0, 0]);

// Storage constants
pub const COLD_STORAGE_ACCESS_COST: U256 = U256([2100, 0, 0, 0]);

// Block constants
pub const LAST_AVAILABLE_BLOCK_LIMIT: U256 = U256([256, 0, 0, 0]);
pub const MAX_BLOCK_GAS_LIMIT: U256 = U256([30_000_000, 0, 0, 0]);
