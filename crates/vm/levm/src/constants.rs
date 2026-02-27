use ethrex_common::{H256, U256};
use std::sync::LazyLock;

pub const WORD_SIZE_IN_BYTES_USIZE: usize = 32;
pub const WORD_SIZE_IN_BYTES_U64: u64 = 32;

pub const SUCCESS: U256 = U256::one();
pub const FAIL: U256 = U256::zero();
pub const WORD_SIZE: usize = 32;

pub const STACK_LIMIT: usize = 1024;

pub const EMPTY_CODE_HASH: H256 = H256([
    0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
    0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
]);

pub const MEMORY_EXPANSION_QUOTIENT: u64 = 512;

// Dedicated gas limit for system calls according to EIPs 2935, 4788, 7002 and 7251
pub const SYS_CALL_GAS_LIMIT: u64 = 30000000;

// Transaction costs in gas
pub const TX_BASE_COST: u64 = 21000;

// https://eips.ethereum.org/EIPS/eip-7825
pub use ethrex_common::constants::POST_OSAKA_GAS_LIMIT_CAP;

pub const MAX_CODE_SIZE: u64 = 0x6000;
pub const INIT_CODE_MAX_SIZE: usize = 49152;

// https://eips.ethereum.org/EIPS/eip-3541
pub const EOF_PREFIX: u8 = 0xef;

pub mod create_opcode {
    use ethrex_common::U256;

    pub const INIT_CODE_WORD_COST: U256 = U256([2, 0, 0, 0]);
    pub const CODE_DEPOSIT_COST: U256 = U256([200, 0, 0, 0]);
    pub const CREATE_BASE_COST: U256 = U256([32000, 0, 0, 0]);
}

pub const VERSIONED_HASH_VERSION_KZG: u8 = 0x01;

// Blob constants
pub const TARGET_BLOB_GAS_PER_BLOCK: u32 = 393216; // TARGET_BLOB_NUMBER_PER_BLOCK * GAS_PER_BLOB
pub const TARGET_BLOB_GAS_PER_BLOCK_PECTRA: u32 = 786432; // TARGET_BLOB_NUMBER_PER_BLOCK * GAS_PER_BLOB

pub const MIN_BASE_FEE_PER_BLOB_GAS: U256 = U256::one();

// WARNING: Do _not_ use the BLOB_BASE_FEE_UPDATE_FRACTION_* family of
// constants as is. Use the `get_blob_base_fee_update_fraction_value`
// function instead
pub const BLOB_BASE_FEE_UPDATE_FRACTION: u64 = 3338477;
pub const BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE: u64 = 5007716; // Defined in [EIP-7691](https://eips.ethereum.org/EIPS/eip-7691)

// WARNING: Do _not_ use the MAX_BLOB_COUNT_* family of constants as
// is. Use the `max_blobs_per_block` function instead
pub const MAX_BLOB_COUNT: u32 = 6;
pub const MAX_BLOB_COUNT_ELECTRA: u32 = 9;
// Max blob count per tx (introduced by Osaka fork)
pub const MAX_BLOB_COUNT_TX: usize = 6;

pub const VALID_BLOB_PREFIXES: [u8; 2] = [0x01, 0x02];

// Block constants
pub const LAST_AVAILABLE_BLOCK_LIMIT: U256 = U256([256, 0, 0, 0]);

// EIP7702 - EOA Load Code
// secp256k1 curve order: FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
pub static SECP256K1_ORDER: LazyLock<U256> = LazyLock::new(|| U256::from_big_endian(&[
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B,
    0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
]));
pub static SECP256K1_ORDER_OVER2: std::sync::LazyLock<U256> =
    LazyLock::new(|| *SECP256K1_ORDER / U256::from(2));
pub const MAGIC: u8 = 0x05;
pub const SET_CODE_DELEGATION_BYTES: [u8; 3] = [0xef, 0x01, 0x00];
// Set the code of authority to be 0xef0100 || address. This is a delegation designation.
// len(SET_CODE_DELEGATION_BYTES) == 3 + len(Address) == 20 -> 23
pub const EIP7702_DELEGATED_CODE_LEN: usize = 23;
pub const PER_AUTH_BASE_COST: u64 = 12500;
pub const PER_EMPTY_ACCOUNT_COST: u64 = 25000;

/// EIP-7708: keccak256('Transfer(address,address,uint256)')
pub const TRANSFER_EVENT_TOPIC: H256 = H256([
    0xdd, 0xf2, 0x52, 0xad, 0x1b, 0xe2, 0xc8, 0x9b, 0x69, 0xc2, 0xb0, 0x68, 0xfc, 0x37, 0x8d, 0xaa,
    0x95, 0x2b, 0xa7, 0xf1, 0x63, 0xc4, 0xa1, 0x16, 0x28, 0xf5, 0x5a, 0x4d, 0xf5, 0x23, 0xb3, 0xef,
]);

/// EIP-7708: keccak256('Selfdestruct(address,uint256)')
pub const SELFDESTRUCT_EVENT_TOPIC: H256 = H256([
    0x4b, 0xfa, 0xba, 0x34, 0x43, 0xc1, 0xa1, 0x83, 0x6c, 0xd3, 0x62, 0x41, 0x8e, 0xdc, 0x67, 0x9f,
    0xc9, 0x6c, 0xae, 0x84, 0x49, 0xcb, 0xef, 0xcc, 0xb6, 0x45, 0x7c, 0xdf, 0x2c, 0x94, 0x30, 0x83,
]);
