/// Compact flat key encoding for direct account/storage lookups.
///
/// Reduces key sizes compared to nibble-path encoding:
/// - Account: 64 nibbles (64B) → 32 bytes (keccak hash)
/// - Storage: 130 nibbles (130B) → 52 bytes (split-address trick from Nethermind)
///
/// The split-address trick for storage keys:
///   `addr_hash[0:4] || keccak(slot)[0:32] || addr_hash[4:20]`
/// Puts the 4-byte address prefix first for locality (all slots for the same
/// contract cluster together), then the full slot hash, then the remaining
/// address bytes for collision resistance.
use ethrex_common::H256;

/// Convert 64 nibble values to an H256 (32 bytes).
/// Each pair of consecutive nibbles is packed into one byte: `(hi << 4) | lo`.
pub fn nibbles_to_h256(nibbles: &[u8]) -> H256 {
    debug_assert_eq!(nibbles.len(), 64);
    let mut bytes = [0u8; 32];
    for i in 0..32 {
        bytes[i] = (nibbles[i * 2] << 4) | nibbles[i * 2 + 1];
    }
    H256(bytes)
}

/// Encode a flat account key: just the raw keccak hash bytes (32B).
pub fn flat_account_key(address_hash: &H256) -> [u8; 32] {
    address_hash.0
}

/// Encode a flat storage key using the split-address trick (52B).
///
/// Layout: `addr_hash[0:4] || slot_hash[0:32] || addr_hash[4:20]`
pub fn flat_storage_key(address_hash: &H256, slot_hash: &H256) -> [u8; 52] {
    let mut key = [0u8; 52];
    key[0..4].copy_from_slice(&address_hash.0[0..4]);
    key[4..36].copy_from_slice(&slot_hash.0[0..32]);
    key[36..52].copy_from_slice(&address_hash.0[4..20]);
    key
}

/// Decode a flat storage key back to (address_hash_prefix, slot_hash).
/// Only the first 4 bytes and last 16 bytes of the address hash are recoverable.
pub fn decode_flat_storage_key(key: &[u8; 52]) -> (H256, H256) {
    let mut addr_hash = H256::zero();
    addr_hash.0[0..4].copy_from_slice(&key[0..4]);
    addr_hash.0[4..20].copy_from_slice(&key[36..52]);
    // bytes 20..32 of addr_hash are lost (truncated), but we only need
    // the prefix for grouping and the slot hash for lookups.

    let mut slot_hash = H256::zero();
    slot_hash.0[0..32].copy_from_slice(&key[4..36]);

    (addr_hash, slot_hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_key_roundtrip() {
        let hash = H256::random();
        let key = flat_account_key(&hash);
        assert_eq!(key, hash.0);
    }

    #[test]
    fn storage_key_layout() {
        let addr = H256::random();
        let slot = H256::random();
        let key = flat_storage_key(&addr, &slot);

        // First 4 bytes are address prefix
        assert_eq!(&key[0..4], &addr.0[0..4]);
        // Next 32 bytes are slot hash
        assert_eq!(&key[4..36], &slot.0[0..32]);
        // Last 16 bytes are remaining address bytes
        assert_eq!(&key[36..52], &addr.0[4..20]);
    }

    #[test]
    fn storage_key_decode() {
        let addr = H256::random();
        let slot = H256::random();
        let key = flat_storage_key(&addr, &slot);
        let (recovered_addr, recovered_slot) = decode_flat_storage_key(&key);

        assert_eq!(recovered_slot, slot);
        assert_eq!(&recovered_addr.0[0..4], &addr.0[0..4]);
        assert_eq!(&recovered_addr.0[4..20], &addr.0[4..20]);
    }

    #[test]
    fn storage_keys_cluster_by_address() {
        let addr = H256::random();
        let slot1 = H256::random();
        let slot2 = H256::random();
        let key1 = flat_storage_key(&addr, &slot1);
        let key2 = flat_storage_key(&addr, &slot2);

        // Same address prefix → keys cluster together in sorted order
        assert_eq!(&key1[0..4], &key2[0..4]);
    }
}
