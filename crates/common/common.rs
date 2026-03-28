// Re-exports from alloy_primitives
pub use alloy_primitives::{Address, Bloom, BloomInput, FixedBytes, U256};

// Type aliases for backward compatibility with ethereum_types naming
pub type H256 = alloy_primitives::B256;
pub type H160 = Address;
pub type H32 = FixedBytes<4>;
pub type H64 = FixedBytes<8>;
pub type H128 = FixedBytes<16>;
pub type H264 = FixedBytes<33>;
pub type H512 = FixedBytes<64>;
pub type Signature = FixedBytes<65>;

// U512 for mulmod (ruint is a transitive dep via alloy-primitives)
pub type U512 = ruint::Uint<512, 8>;

/// Extension trait providing ethereum_types-compatible API on alloy U256
pub trait U256Ext {
    fn zero() -> Self;
    fn one() -> Self;
    fn from_u64(value: u64) -> Self;
    fn from_big_endian(bytes: &[u8]) -> Self;
    fn to_big_endian(&self) -> [u8; 32];
    fn low_u64(&self) -> u64;
    fn low_u128(&self) -> u128;
    fn low_u32(&self) -> u32;
    fn as_u64(&self) -> u64;
    fn as_u128(&self) -> u128;
    fn bits(&self) -> usize;
    fn full_mul(&self, other: Self) -> U512;
    fn from_dec_str(s: &str) -> Result<Self, ruint::ParseError>
    where
        Self: Sized;
    fn significant_bits(&self) -> usize;
}

impl U256Ext for U256 {
    #[inline]
    fn zero() -> Self {
        Self::ZERO
    }

    #[inline]
    fn one() -> Self {
        Self::from_limbs([1, 0, 0, 0])
    }

    #[inline]
    fn from_u64(value: u64) -> Self {
        Self::from_limbs([value, 0, 0, 0])
    }

    #[inline]
    fn from_big_endian(bytes: &[u8]) -> Self {
        Self::from_be_slice(bytes)
    }

    #[inline]
    fn to_big_endian(&self) -> [u8; 32] {
        self.to_be_bytes::<32>()
    }

    #[inline]
    fn low_u64(&self) -> u64 {
        self.as_limbs()[0]
    }

    #[inline]
    fn low_u128(&self) -> u128 {
        u128::from(self.as_limbs()[0]) | (u128::from(self.as_limbs()[1]) << 64)
    }

    #[inline]
    fn low_u32(&self) -> u32 {
        (self.as_limbs()[0] & 0xFFFF_FFFF) as u32
    }

    #[inline]
    fn as_u64(&self) -> u64 {
        self.to::<u64>()
    }

    #[inline]
    fn as_u128(&self) -> u128 {
        self.to::<u128>()
    }

    #[inline]
    fn bits(&self) -> usize {
        self.bit_len()
    }

    #[inline]
    fn full_mul(&self, other: Self) -> U512 {
        U512::from(*self) * U512::from(other)
    }

    #[inline]
    fn from_dec_str(s: &str) -> Result<Self, ruint::ParseError> {
        Self::from_str_radix(s, 10)
    }

    #[inline]
    fn significant_bits(&self) -> usize {
        self.bit_len()
    }
}

/// Extension trait providing ethereum_types-compatible API on alloy B256 (H256)
pub trait H256Ext {
    fn from_uint(value: &U256) -> Self;
    fn as_bytes(&self) -> &[u8];
    fn as_fixed_bytes(&self) -> &[u8; 32];
    fn to_fixed_bytes(&self) -> [u8; 32];
    fn from_low_u64_be(value: u64) -> Self;
}

impl H256Ext for H256 {
    #[inline]
    fn from_uint(value: &U256) -> Self {
        Self::from(value.to_be_bytes::<32>())
    }

    #[inline]
    fn as_bytes(&self) -> &[u8] {
        self.as_slice()
    }

    #[inline]
    fn as_fixed_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[inline]
    fn to_fixed_bytes(&self) -> [u8; 32] {
        self.0
    }

    #[inline]
    fn from_low_u64_be(value: u64) -> Self {
        let mut bytes = [0u8; 32];
        bytes[24..32].copy_from_slice(&value.to_be_bytes());
        Self::from(bytes)
    }
}

/// Extension trait providing ethereum_types-compatible API on alloy Address (H160)
pub trait AddressExt {
    fn as_bytes(&self) -> &[u8];
    fn as_fixed_bytes(&self) -> &[u8; 20];
    fn to_fixed_bytes(&self) -> [u8; 20];
    fn from_low_u64_be(value: u64) -> Self;
}

impl AddressExt for Address {
    #[inline]
    fn as_bytes(&self) -> &[u8] {
        self.as_slice()
    }

    #[inline]
    fn as_fixed_bytes(&self) -> &[u8; 20] {
        &self.0 .0
    }

    #[inline]
    fn to_fixed_bytes(&self) -> [u8; 20] {
        self.0 .0
    }

    #[inline]
    fn from_low_u64_be(value: u64) -> Self {
        let mut bytes = [0u8; 20];
        bytes[12..20].copy_from_slice(&value.to_be_bytes());
        Self::from(bytes)
    }
}

/// Replaces ethereum_types::BigEndianHash — converts between U256 and H256
pub trait BigEndianHash {
    fn from_uint(value: &U256) -> H256;
    fn into_uint(hash: &H256) -> U256;
}

impl BigEndianHash for H256 {
    fn from_uint(value: &U256) -> H256 {
        H256::from(value.to_be_bytes::<32>())
    }

    fn into_uint(hash: &H256) -> U256 {
        U256::from_be_bytes(hash.0)
    }
}

pub mod constants;
pub mod serde_utils;
pub mod types;
pub mod validation;
pub use bytes::Bytes;
pub mod base64;
pub use ethrex_trie::{TrieLogger, TrieWitness};
pub mod errors;
pub mod evm;
pub mod fd_limit;
pub mod genesis_utils;
pub mod rkyv_utils;
pub mod tracing;
pub mod utils;

pub use errors::InvalidBlockError;
pub use ethrex_crypto::CryptoError;
pub use validation::{
    get_total_blob_gas, validate_block_access_list_hash, validate_block_access_list_size,
    validate_block_pre_execution, validate_gas_used, validate_header_bal_indices,
    validate_receipts_root, validate_requests_hash,
};
