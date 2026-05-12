// === YELLOW PAPER constants ===

/// Base gas cost for each non contract creating transaction
pub const TX_GAS_COST: u64 = 21000;

/// Base gas cost for each contract creating transaction
pub const TX_CREATE_GAS_COST: u64 = 53000;

// Gas cost for each zero byte on transaction data
pub const TX_DATA_ZERO_GAS_COST: u64 = 4;

// Gas cost for each init code word on transaction data
pub const TX_INIT_CODE_WORD_GAS_COST: u64 = 2;

// Gas cost for each address specified on access lists
pub const TX_ACCESS_LIST_ADDRESS_GAS: u64 = 2400;

// Gas cost for each storage key specified on access lists
pub const TX_ACCESS_LIST_STORAGE_KEY_GAS: u64 = 1900;

// Gas cost for each non zero byte on transaction data
pub const TX_DATA_NON_ZERO_GAS: u64 = 68;

// === EIP-170 constants ===

// Max bytecode size
pub const MAX_CODE_SIZE: u32 = 0x6000;
// EIP-7954 (Amsterdam): increased max bytecode size
pub const AMSTERDAM_MAX_CODE_SIZE: u32 = 0x8000;

// === EIP-3860 constants ===

// Max contract creation bytecode size
pub const MAX_INITCODE_SIZE: u32 = 2 * MAX_CODE_SIZE;
// EIP-7954 (Amsterdam): increased max initcode size
pub const AMSTERDAM_MAX_INITCODE_SIZE: u32 = 2 * AMSTERDAM_MAX_CODE_SIZE;

// Max non-contract creation bytecode size
pub const MAX_TRANSACTION_DATA_SIZE: u32 = 4 * 32 * 1024; // 128 Kb

// === EIP-2028 constants ===

// Gas cost for each non zero byte on transaction data
pub const TX_DATA_NON_ZERO_GAS_EIP2028: u64 = 16;

// === EIP-4844 constants ===

pub const GAS_LIMIT_BOUND_DIVISOR: u64 = 1024;

pub const MIN_GAS_LIMIT: u64 = 5000;

// === EIP-7825 constants ===
// https://eips.ethereum.org/EIPS/eip-7825
pub const POST_OSAKA_GAS_LIMIT_CAP: u64 = 16777216;

// === Mempool sweep defaults ===

use std::time::Duration;

/// Default maximum age of a mempool transaction before the periodic sweep
/// evicts it. Transactions older than this are dropped regardless of pool
/// occupancy.
pub const DEFAULT_MEMPOOL_LIFETIME: Duration = Duration::from_secs(3 * 60 * 60);

/// Default maximum gap allowed between a sender's top pending nonce and
/// their on-chain nonce before the dormancy sweep considers them stalled.
pub const DEFAULT_MAX_NONCE_GAP: u64 = 64;

/// Default dormancy window for the nonce-gap sweep. A sender must have made
/// no on-chain progress for at least this long (i.e. all their pool entries
/// are older than this) before they are eligible for eviction.
pub const DEFAULT_DORMANCY: Duration = Duration::from_secs(3 * 60 * 60);

/// How often the periodic mempool sweep runs.
pub const MEMPOOL_SWEEP_INTERVAL: Duration = Duration::from_secs(60);
