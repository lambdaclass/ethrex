// Fee related
pub const ELASTICITY_MULTIPLIER: u64 = 2;
pub const BASE_FEE_MAX_CHANGE_DENOMINATOR: u128 = 8;
pub const GAS_LIMIT_ADJUSTMENT_FACTOR: u64 = 1024;
pub const GAS_LIMIT_MINIMUM: u64 = 5000;
pub const DEFAULT_BUILDER_GAS_CEIL: u64 = 30_000_000;
pub const GWEI_TO_WEI: u64 = 1_000_000_000;
pub const INITIAL_BASE_FEE: u64 = 1_000_000_000; //Initial base fee as defined in [EIP-1559](https://eips.ethereum.org/EIPS/eip-1559)
pub const MIN_BASE_FEE_PER_BLOB_GAS: u64 = 1; // Defined in [EIP-4844](https://eips.ethereum.org/EIPS/eip-4844)
pub const BLOB_BASE_FEE_UPDATE_FRACTION: u64 = 3338477; // Defined in [EIP-4844](https://eips.ethereum.org/EIPS/eip-4844)
pub const VERSIONED_HASH_VERSION_KZG: u8 = 0x01; // Defined in [EIP-4844](https://eips.ethereum.org/EIPS/eip-4844)
/// Minimum tip, obtained from geth's default miner config (https://github.com/ethereum/go-ethereum/blob/f750117ad19d623622cc4a46ea361a716ba7407e/miner/miner.go#L56)
/// TODO: This should be configurable along with the tip filter on https://github.com/lambdaclass/ethrex/issues/680
pub const MIN_GAS_TIP: u64 = 1000000;

// Blob size related
// Defined in [EIP-4844](https://eips.ethereum.org/EIPS/eip-4844)
pub const BYTES_PER_FIELD_ELEMENT: usize = 32;
pub const FIELD_ELEMENTS_PER_BLOB: usize = 4096;
pub const BYTES_PER_BLOB: usize = BYTES_PER_FIELD_ELEMENT * FIELD_ELEMENTS_PER_BLOB;
pub const BYTES_PER_BLOB_F64: f64 = BYTES_PER_BLOB as f64;
/// The maximum number of bytes that can be "safely" stored in a blob. This is, prepend
/// a zero byte for every 32 bytes of data to ensure they not exceed the field modulus.
pub const SAFE_BYTES_PER_BLOB: usize = BYTES_PER_BLOB * 31 / 32;
