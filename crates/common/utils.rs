use crate::{H256, U256};
use ethrex_crypto::keccak::keccak_hash;
use hex::FromHexError;

pub const ZERO_U256: U256 = U256::ZERO;

pub fn u256_from_big_endian(slice: &[u8]) -> U256 {
    U256::from_be_slice(slice)
}

pub fn u256_from_big_endian_const<const N: usize>(slice: [u8; N]) -> U256 {
    const { assert!(N <= 32, "N must be less or equal to 32") };

    let mut padded = [0u8; 32];
    padded[32 - N..32].copy_from_slice(&slice);

    U256::from_be_bytes(padded)
}

#[inline(always)]
pub fn u256_to_big_endian(value: U256) -> [u8; 32] {
    value.to_be_bytes::<32>()
}

#[inline(always)]
pub fn u256_to_h256(value: U256) -> H256 {
    H256::from(value.to_be_bytes::<32>())
}

pub fn decode_hex(hex: &str) -> Result<Vec<u8>, FromHexError> {
    let trimmed = hex.strip_prefix("0x").unwrap_or(hex);
    hex::decode(trimmed)
}

pub fn keccak(data: impl AsRef<[u8]>) -> H256 {
    H256::from(keccak_hash(data))
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
