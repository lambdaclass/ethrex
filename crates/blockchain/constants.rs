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
pub const MAX_CODE_SIZE: usize = 0x6000;

// === EIP-3860 constants ===

// Max contract creation bytecode size
pub const MAX_INITCODE_SIZE: usize = 2 * MAX_CODE_SIZE;

// === EIP-2028 constants ===

// Gas cost for each non zero byte on transaction data
pub const TX_DATA_NON_ZERO_GAS_EIP2028: u64 = 16;

// === EIP-4844 constants ===

/// Gas consumption of a single data blob (== blob byte size).
pub const GAS_PER_BLOB: u64 = 1 << 17;

/// Target gas consumption for data blobs per block.
pub const TARGET_BLOB_GAS_PER_BLOCK: u64 = 393216;

/// Target number of the blob per block.
pub const TARGET_BLOB_NUMBER_PER_BLOCK: u64 = 3;

/// Max number of blobs per block
pub const MAX_BLOB_NUMBER_PER_BLOCK: u64 = 2 * TARGET_BLOB_NUMBER_PER_BLOCK;

/// Maximum consumable blob gas for data blobs per block.
pub const MAX_BLOB_GAS_PER_BLOCK: u64 = MAX_BLOB_NUMBER_PER_BLOCK * GAS_PER_BLOB;

// Minimum base fee per blob
pub const MIN_BASE_FEE_PER_BLOB_GAS: u64 = 1;

pub const GAS_LIMIT_BOUND_DIVISOR: u64 = 1024;

pub const MIN_GAS_LIMIT: u64 = 5000;
