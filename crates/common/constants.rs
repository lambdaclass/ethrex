use ethereum_types::Address;

// === EIP-4844 constants ===

/// Gas consumption of a single data blob (== blob byte size).
pub const GAS_PER_BLOB: u64 = 1 << 17;

// Minimum base fee per blob
pub const MIN_BASE_FEE_PER_BLOB_GAS: u64 = 1;

// === General use constants ===

lazy_static::lazy_static! {
    pub static ref MAINNET_DEPOSIT_CONTRACT_ADDRESS: Address = Address::from_slice(&hex::decode("00000000219ab540356cbb839cbe05303d7705fa").unwrap());
}
