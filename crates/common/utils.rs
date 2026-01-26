use crate::H256;
use ethereum_types::U256;
use ethrex_crypto::keccak::keccak_hash;
use hex::FromHexError;

pub const ZERO_U256: U256 = U256([0, 0, 0, 0]);

/// Converts a big endian slice to a u256, faster than `u256::from_big_endian`.
#[inline(always)]
pub fn u256_from_big_endian(slice: &[u8]) -> U256 {
    let mut padded = [0u8; 32];
    padded[32 - slice.len()..32].copy_from_slice(slice);

    #[expect(clippy::unwrap_used)]
    U256([
        u64::from_be_bytes(padded[24..32].try_into().unwrap()),
        u64::from_be_bytes(padded[16..24].try_into().unwrap()),
        u64::from_be_bytes(padded[8..16].try_into().unwrap()),
        u64::from_be_bytes(padded[0..8].try_into().unwrap()),
    ])
}

/// Converts a constant big endian slice to a u256, faster than `u256::from_big_endian` and `u256_from_big_endian`.
///
/// Note: N should not exceed 32.
#[inline(always)]
pub fn u256_from_big_endian_const<const N: usize>(slice: [u8; N]) -> U256 {
    const { assert!(N <= 32, "N must be less or equal to 32") };

    // Fast path: N=32 needs no padding
    if N == 32 {
        // SAFETY: When N=32, slice is exactly [u8; 32].
        // Pointer casts are valid because u8 has alignment 1.
        #[expect(unsafe_code)]
        return unsafe {
            let ptr = slice.as_ptr();
            U256([
                u64::from_be_bytes(*ptr.add(24).cast::<[u8; 8]>()),
                u64::from_be_bytes(*ptr.add(16).cast::<[u8; 8]>()),
                u64::from_be_bytes(*ptr.add(8).cast::<[u8; 8]>()),
                u64::from_be_bytes(*ptr.cast::<[u8; 8]>()),
            ])
        };
    }

    // General case: N < 32 needs zero-padding on the left
    let mut padded = [0u8; 32];
    padded[32 - N..32].copy_from_slice(&slice);

    #[expect(clippy::unwrap_used)]
    U256([
        u64::from_be_bytes(padded[24..32].try_into().unwrap()),
        u64::from_be_bytes(padded[16..24].try_into().unwrap()),
        u64::from_be_bytes(padded[8..16].try_into().unwrap()),
        u64::from_be_bytes(padded[0..8].try_into().unwrap()),
    ])
}

/// Converts a U256 to a big endian slice.
#[inline(always)]
pub fn u256_to_big_endian(value: U256) -> [u8; 32] {
    let mut bytes = [0u8; 32];

    for i in 0..4 {
        let u64_be = value.0[4 - i - 1].to_be_bytes();
        bytes[8 * i..(8 * i + 8)].copy_from_slice(&u64_be);
    }

    bytes
}

#[inline(always)]
pub fn u256_to_h256(value: U256) -> H256 {
    H256(u256_to_big_endian(value))
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
