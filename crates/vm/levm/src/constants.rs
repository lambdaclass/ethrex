use ethrex_core::{H256, U256};

pub const WORD_SIZE_IN_BYTES: U256 = U256([32, 0, 0, 0]);
pub const WORD_SIZE_IN_BYTES_USIZE: usize = 32;

pub const SUCCESS_FOR_CALL: i32 = 1;
pub const REVERT_FOR_CALL: i32 = 0;
pub const HALT_FOR_CALL: i32 = 2;
pub const SUCCESS_FOR_RETURN: i32 = 1;
pub const CREATE_DEPLOYMENT_FAIL: U256 = U256::zero();
pub const WORD_SIZE: usize = 32;

pub const STACK_LIMIT: usize = 1024;

pub const GAS_REFUND_DENOMINATOR: u64 = 5;

pub const EMPTY_CODE_HASH: H256 = H256([
    0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
    0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
]);

pub const MEMORY_EXPANSION_QUOTIENT: usize = 512;

// Transaction costs in gas (in wei)
pub const TX_BASE_COST: U256 = U256([21000, 0, 0, 0]);

pub const MAX_CODE_SIZE: usize = 0x6000;
pub const INIT_CODE_MAX_SIZE: usize = 49152;
pub const MAX_CREATE_CODE_SIZE: usize = 2 * MAX_CODE_SIZE;

pub const INVALID_CONTRACT_PREFIX: u8 = 0xef;

pub mod create_opcode {
    use ethrex_core::U256;

    pub const INIT_CODE_WORD_COST: U256 = U256([2, 0, 0, 0]);
    pub const CODE_DEPOSIT_COST: U256 = U256([200, 0, 0, 0]);
    pub const CREATE_BASE_COST: U256 = U256([32000, 0, 0, 0]);
}

pub const VERSIONED_HASH_VERSION_KZG: u8 = 0x01;
pub const MAX_BLOB_NUMBER_PER_BLOCK: usize = 6;

// Blob constants
pub const TARGET_BLOB_GAS_PER_BLOCK: U256 = U256([393216, 0, 0, 0]); // TARGET_BLOB_NUMBER_PER_BLOCK * GAS_PER_BLOB
pub const MIN_BASE_FEE_PER_BLOB_GAS: u64 = 1;
pub const BLOB_BASE_FEE_UPDATE_FRACTION: u64 = 3338477;
pub const MAX_BLOB_COUNT: usize = 6;
pub const VALID_BLOB_PREFIXES: [u8; 2] = [0x01, 0x02];

// Block constants
pub const LAST_AVAILABLE_BLOCK_LIMIT: U256 = U256([256, 0, 0, 0]);
pub const MAX_BLOCK_GAS_LIMIT: U256 = U256([30_000_000, 0, 0, 0]);
