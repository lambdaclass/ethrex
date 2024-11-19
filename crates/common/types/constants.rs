// Fee related
pub const ELASTICITY_MULTIPLIER: u64 = 2;
pub const BASE_FEE_MAX_CHANGE_DENOMINATOR: u64 = 8;
pub const GAS_LIMIT_ADJUSTMENT_FACTOR: u64 = 1024;
pub const GAS_LIMIT_MINIMUM: u64 = 5000;
pub const GWEI_TO_WEI: u64 = 1_000_000_000;
pub const INITIAL_BASE_FEE: u64 = 1_000_000_000; //Initial base fee as defined in [EIP-1559](https://eips.ethereum.org/EIPS/eip-1559)
pub const MIN_BASE_FEE_PER_BLOB_GAS: u64 = 1; // Defined in [EIP-4844](https://eips.ethereum.org/EIPS/eip-4844)
pub const BLOB_BASE_FEE_UPDATE_FRACTION: u64 = 3338477; // Defined in [EIP-4844](https://eips.ethereum.org/EIPS/eip-4844)
pub const VERSIONED_HASH_VERSION_KZG: u8 = 0x01; // Defined in [EIP-4844](https://eips.ethereum.org/EIPS/eip-4844)

// Blob size related
// Defined in [EIP-4844](https://eips.ethereum.org/EIPS/eip-4844)
pub const BYTES_PER_FIELD_ELEMENT: usize = 32;
pub const FIELD_ELEMENTS_PER_BLOB: usize = 4096;
pub const BYTES_PER_BLOB: usize = BYTES_PER_FIELD_ELEMENT * FIELD_ELEMENTS_PER_BLOB;
