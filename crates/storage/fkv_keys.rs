//! FlatKeyValue binary key encoding functions.
//!
//! This module provides functions for encoding and decoding FlatKeyValue keys
//! using a Nethermind-style binary format, which is more space-efficient than
//! the nibble-based format.
//!
//! ## Key Formats
//!
//! - **Account Key**: 20 bytes = first 20 bytes of keccak256(address)
//! - **Storage Key**: 52 bytes = [4B addr prefix][32B slot hash][16B addr suffix]
//!
//! The storage key format interleaves address and slot hash bytes to maintain
//! ordering properties while reducing key size.

use ethrex_common::{H256, U256};

/// Size of account FKV keys in bytes (truncated address hash)
pub const ACCOUNT_FKV_KEY_SIZE: usize = 20;

/// Size of storage FKV keys in bytes
/// Format: [4B addr prefix][32B slot hash][16B addr suffix]
pub const STORAGE_FKV_KEY_SIZE: usize = 52;

/// Create a binary FKV key for an account.
///
/// Takes the first 20 bytes of the address hash (keccak256(address)).
/// This provides sufficient uniqueness while reducing key size from 65 nibbles
/// (32.5 bytes when packed) to exactly 20 bytes.
#[inline]
pub fn account_fkv_key(address_hash: &H256) -> [u8; ACCOUNT_FKV_KEY_SIZE] {
    let mut key = [0u8; ACCOUNT_FKV_KEY_SIZE];
    key.copy_from_slice(&address_hash.as_bytes()[..ACCOUNT_FKV_KEY_SIZE]);
    key
}

/// Create a binary FKV key for a storage slot.
///
/// Format: [4B addr prefix][32B slot hash][16B addr suffix]
///
/// This interleaved format:
/// 1. Groups storage slots by account (first 4 bytes are account prefix)
/// 2. Includes full slot hash for uniqueness (32 bytes)
/// 3. Includes remaining address bytes for collision resistance (16 bytes)
///
/// Total: 4 + 32 + 16 = 52 bytes
#[inline]
pub fn storage_fkv_key(address_hash: &H256, slot_hash: &H256) -> [u8; STORAGE_FKV_KEY_SIZE] {
    let mut key = [0u8; STORAGE_FKV_KEY_SIZE];
    let addr_bytes = address_hash.as_bytes();
    let slot_bytes = slot_hash.as_bytes();

    // First 4 bytes: address prefix
    key[..4].copy_from_slice(&addr_bytes[..4]);
    // Next 32 bytes: full slot hash
    key[4..36].copy_from_slice(slot_bytes);
    // Last 16 bytes: address suffix (bytes 4-19 of address hash, completing the 20-byte truncation)
    key[36..52].copy_from_slice(&addr_bytes[4..20]);

    key
}

/// Extract address hash prefix and slot hash from a storage FKV key.
///
/// Returns (address_hash_truncated, slot_hash) where address_hash_truncated
/// contains the first 20 bytes of the original address hash.
#[inline]
pub fn decode_storage_fkv_key(key: &[u8; STORAGE_FKV_KEY_SIZE]) -> ([u8; 20], H256) {
    let mut addr_truncated = [0u8; 20];
    // Reconstruct the truncated address hash
    addr_truncated[..4].copy_from_slice(&key[..4]);
    addr_truncated[4..20].copy_from_slice(&key[36..52]);

    let mut slot_bytes = [0u8; 32];
    slot_bytes.copy_from_slice(&key[4..36]);

    (addr_truncated, H256::from(slot_bytes))
}

/// Strip leading zeros from a U256 value for compact storage.
///
/// Storage values are U256 but often have many leading zeros.
/// Stripping them reduces storage significantly.
#[inline]
pub fn strip_leading_zeros(value: &[u8]) -> Vec<u8> {
    // Find first non-zero byte
    let first_nonzero = value.iter().position(|&b| b != 0);
    match first_nonzero {
        Some(pos) => value[pos..].to_vec(),
        None => vec![], // All zeros -> empty vec
    }
}

/// Restore a U256 from stripped bytes.
///
/// The stripped bytes are right-aligned into a 32-byte array.
#[inline]
pub fn restore_u256(stripped: &[u8]) -> U256 {
    if stripped.is_empty() {
        return U256::zero();
    }
    if stripped.len() > 32 {
        // Shouldn't happen with valid data, but handle gracefully
        return U256::from_big_endian(&stripped[stripped.len() - 32..]);
    }
    let mut bytes = [0u8; 32];
    bytes[32 - stripped.len()..].copy_from_slice(stripped);
    U256::from_big_endian(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::H256;
    use std::str::FromStr;

    #[test]
    fn test_account_fkv_key() {
        let hash = H256::from_str(
            "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        )
        .unwrap();
        let key = account_fkv_key(&hash);
        assert_eq!(key.len(), ACCOUNT_FKV_KEY_SIZE);
        assert_eq!(&key[..], &hash.as_bytes()[..20]);
    }

    #[test]
    fn test_storage_fkv_key() {
        let addr_hash = H256::from_str(
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa000000000000000000000000",
        )
        .unwrap();
        let slot_hash = H256::from_str(
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap();

        let key = storage_fkv_key(&addr_hash, &slot_hash);
        assert_eq!(key.len(), STORAGE_FKV_KEY_SIZE);

        // Verify structure: [4B addr][32B slot][16B addr]
        assert_eq!(&key[..4], &addr_hash.as_bytes()[..4]);
        assert_eq!(&key[4..36], slot_hash.as_bytes());
        assert_eq!(&key[36..52], &addr_hash.as_bytes()[4..20]);
    }

    #[test]
    fn test_decode_storage_fkv_key() {
        let addr_hash = H256::from_str(
            "0x1234567890abcdef1234567890abcdef12345678000000000000000000000000",
        )
        .unwrap();
        let slot_hash = H256::from_str(
            "0xfedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321",
        )
        .unwrap();

        let key = storage_fkv_key(&addr_hash, &slot_hash);
        let (addr_truncated, decoded_slot) = decode_storage_fkv_key(&key);

        assert_eq!(&addr_truncated[..], &addr_hash.as_bytes()[..20]);
        assert_eq!(decoded_slot, slot_hash);
    }

    #[test]
    fn test_strip_leading_zeros() {
        // Normal case
        let value = [0, 0, 0, 1, 2, 3];
        assert_eq!(strip_leading_zeros(&value), vec![1, 2, 3]);

        // No leading zeros
        let value = [1, 2, 3];
        assert_eq!(strip_leading_zeros(&value), vec![1, 2, 3]);

        // All zeros
        let value = [0, 0, 0];
        assert_eq!(strip_leading_zeros(&value), Vec::<u8>::new());

        // Empty input
        let value: [u8; 0] = [];
        assert_eq!(strip_leading_zeros(&value), Vec::<u8>::new());
    }

    #[test]
    fn test_restore_u256() {
        // Normal case
        let stripped = vec![1, 2, 3];
        let restored = restore_u256(&stripped);
        assert_eq!(restored, U256::from(0x010203u64));

        // Empty (zero)
        let stripped: Vec<u8> = vec![];
        let restored = restore_u256(&stripped);
        assert_eq!(restored, U256::zero());

        // Full 32 bytes
        let mut full = vec![0u8; 32];
        full[31] = 42;
        let restored = restore_u256(&full);
        assert_eq!(restored, U256::from(42u64));
    }

    #[test]
    fn test_roundtrip_u256() {
        let values = [
            U256::zero(),
            U256::one(),
            U256::from(255u64),
            U256::from(0x1234567890abcdefu64),
            U256::MAX,
        ];

        for value in values {
            let bytes = value.to_big_endian();
            let stripped = strip_leading_zeros(&bytes);
            let restored = restore_u256(&stripped);
            assert_eq!(restored, value);
        }
    }
}
