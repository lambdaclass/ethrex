use ethrex_common::{H256, U256};
use k256::{
    elliptic_curve::{bigint::Encoding, Curve},
    Secp256k1,
};
use std::sync::LazyLock;

pub const WORD_SIZE_IN_BYTES: U256 = U256([32, 0, 0, 0]);
pub const WORD_SIZE_IN_BYTES_USIZE: usize = 32;
pub const WORD_SIZE_IN_BYTES_U64: u64 = 32;

pub const SUCCESS_FOR_CALL: U256 = U256::one();
pub const REVERT_FOR_CALL: U256 = U256::zero();
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
pub const TX_BASE_COST: u64 = 21000;

pub const MAX_CODE_SIZE: usize = 0x6000;
pub const INIT_CODE_MAX_SIZE: usize = 49152;
pub const MAX_CREATE_CODE_SIZE: usize = 2 * MAX_CODE_SIZE;

pub const INVALID_CONTRACT_PREFIX: u8 = 0xef;

pub mod create_opcode {
    use ethrex_common::U256;

    pub const INIT_CODE_WORD_COST: U256 = U256([2, 0, 0, 0]);
    pub const CODE_DEPOSIT_COST: U256 = U256([200, 0, 0, 0]);
    pub const CREATE_BASE_COST: U256 = U256([32000, 0, 0, 0]);
}

pub const VERSIONED_HASH_VERSION_KZG: u8 = 0x01;

// Blob constants
pub const TARGET_BLOB_GAS_PER_BLOCK: u64 = 393216; // TARGET_BLOB_NUMBER_PER_BLOCK * GAS_PER_BLOB
pub const TARGET_BLOB_GAS_PER_BLOCK_PECTRA: u64 = 786432; // TARGET_BLOB_NUMBER_PER_BLOCK * GAS_PER_BLOB

pub const MIN_BASE_FEE_PER_BLOB_GAS: U256 = U256::one();

// WARNING: Do _not_ use the BLOB_BASE_FEE_UPDATE_FRACTION_* family of
// constants as is. Use the `get_blob_base_fee_update_fraction_value`
// function instead
pub const BLOB_BASE_FEE_UPDATE_FRACTION: u64 = 3338477;
pub const BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE: u64 = 5007716; // Defined in [EIP-7691](https://eips.ethereum.org/EIPS/eip-7691)

// WARNING: Do _not_ use the MAX_BLOB_COUNT_* family of constants as
// is. Use the `max_blobs_per_block` function instead
pub const MAX_BLOB_COUNT: u64 = 6;
pub const MAX_BLOB_COUNT_ELECTRA: u64 = 9;

pub const VALID_BLOB_PREFIXES: [u8; 2] = [0x01, 0x02];

// Block constants
pub const LAST_AVAILABLE_BLOCK_LIMIT: U256 = U256([256, 0, 0, 0]);
pub const MAX_BLOCK_GAS_LIMIT: U256 = U256([30_000_000, 0, 0, 0]);

// EIP7702 - EOA Load Code
pub static SECP256K1_ORDER: LazyLock<U256> =
    LazyLock::new(|| U256::from_big_endian(&Secp256k1::ORDER.to_be_bytes()));
pub static SECP256K1_ORDER_OVER2: LazyLock<U256> =
    LazyLock::new(|| *SECP256K1_ORDER / U256::from(2));
pub const MAGIC: u8 = 0x05;
pub const SET_CODE_DELEGATION_BYTES: [u8; 3] = [0xef, 0x01, 0x00];
// Set the code of authority to be 0xef0100 || address. This is a delegation designation.
// len(SET_CODE_DELEGATION_BYTES) == 3 + len(Address) == 20 -> 23
pub const EIP7702_DELEGATED_CODE_LEN: usize = 23;
pub const PER_AUTH_BASE_COST: u64 = 12500;
pub const PER_EMPTY_ACCOUNT_COST: u64 = 25000;
