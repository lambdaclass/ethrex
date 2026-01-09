use crate::{H256, U256, U512};
use ethrex_crypto::keccak::keccak_hash;
use hex::FromHexError;

/// Trait for conversion between hash types and unsigned integers.
/// This provides similar functionality to ethereum_types::BigEndianHash
/// but works with ruint's U256.
pub trait BigEndianHash<T> {
    /// Convert from a U256 value
    fn from_uint(val: &T) -> Self;
    /// Convert to a U256 value
    fn into_uint(&self) -> T;
}

impl BigEndianHash<U256> for H256 {
    fn from_uint(val: &U256) -> Self {
        H256(val.to_be_bytes::<32>())
    }

    fn into_uint(&self) -> U256 {
        U256::from_be_bytes(self.0)
    }
}

impl BigEndianHash<U512> for crate::H512 {
    fn from_uint(val: &U512) -> Self {
        crate::H512(val.to_be_bytes::<64>())
    }

    fn into_uint(&self) -> U512 {
        U512::from_be_bytes(self.0)
    }
}

pub const ZERO_U256: U256 = U256::ZERO;

/// Converts a big endian slice to a U256.
pub fn u256_from_big_endian(slice: &[u8]) -> U256 {
    let mut padded = [0u8; 32];
    padded[32 - slice.len()..32].copy_from_slice(slice);
    U256::from_be_bytes(padded)
}

/// Converts a constant big endian slice to a U256.
///
/// Note: N should not exceed 32.
pub fn u256_from_big_endian_const<const N: usize>(slice: [u8; N]) -> U256 {
    const { assert!(N <= 32, "N must be less or equal to 32") };

    let mut padded = [0u8; 32];
    padded[32 - N..32].copy_from_slice(&slice);
    U256::from_be_bytes(padded)
}

/// Converts a U256 to a big endian byte array.
#[inline(always)]
pub fn u256_to_big_endian(value: U256) -> [u8; 32] {
    value.to_be_bytes::<32>()
}

#[inline(always)]
pub fn u256_to_h256(value: U256) -> H256 {
    H256(u256_to_big_endian(value))
}

#[inline(always)]
pub fn h256_to_u256(value: H256) -> U256 {
    U256::from_be_bytes(value.0)
}

/// Extension trait for U256 to provide conversion methods compatible with ethereum_types API
pub trait U256Ext {
    fn as_u64(&self) -> u64;
    fn as_u32(&self) -> u32;
    fn as_usize(&self) -> usize;
    fn low_u64(&self) -> u64;
    fn low_u32(&self) -> u32;
}

impl U256Ext for U256 {
    /// Returns the low 64 bits, saturating to u64::MAX if the value is larger
    #[inline(always)]
    fn as_u64(&self) -> u64 {
        u64::try_from(*self).unwrap_or(u64::MAX)
    }

    /// Returns the low 32 bits, saturating to u32::MAX if the value is larger
    #[inline(always)]
    fn as_u32(&self) -> u32 {
        u32::try_from(*self).unwrap_or(u32::MAX)
    }

    /// Returns the low bits as usize, saturating to usize::MAX if the value is larger
    #[inline(always)]
    fn as_usize(&self) -> usize {
        usize::try_from(*self).unwrap_or(usize::MAX)
    }

    /// Returns the low 64 bits (no saturation, just truncation)
    #[inline(always)]
    fn low_u64(&self) -> u64 {
        self.as_limbs()[0]
    }

    /// Returns the low 32 bits (no saturation, just truncation)
    #[inline(always)]
    fn low_u32(&self) -> u32 {
        self.as_limbs()[0] as u32
    }
}

pub fn decode_hex(hex: &str) -> Result<Vec<u8>, FromHexError> {
    let trimmed = hex.strip_prefix("0x").unwrap_or(hex);
    hex::decode(trimmed)
}

pub fn keccak(data: impl AsRef<[u8]>) -> H256 {
    H256(keccak_hash(data))
}

// Allocation-free operations on arrays.
///
/// Truncates an array of size N to size M.
/// Fails compilation if N < M.
pub fn truncate_array<const N: usize, const M: usize>(data: [u8; N]) -> [u8; M] {
    const { assert!(M <= N) };
    let mut res = [0u8; M];
    res.copy_from_slice(&data[..M]);
    res
}

#[cfg(test)]
mod test {
    use crate::U256;
    use crate::utils::{u256_from_big_endian, u256_to_big_endian};

    #[test]
    fn u256_to_big_endian_test() {
        // Test with ONE
        let bytes = u256_to_big_endian(U256::from(1u64));
        let mut expected = [0u8; 32];
        expected[31] = 1;
        assert_eq!(bytes, expected);

        // Test with MAX
        let bytes = u256_to_big_endian(U256::MAX);
        assert_eq!(bytes, [0xff; 32]);
    }

    #[test]
    fn u256_roundtrip_test() {
        let original = U256::from(0x123456789ABCDEFu64);
        let bytes = u256_to_big_endian(original);
        let recovered = u256_from_big_endian(&bytes);
        assert_eq!(original, recovered);
    }
}
