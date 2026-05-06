use ethrex_common::H256;

/// The empty binary trie root. All-zeros, distinct from `EMPTY_TRIE_HASH` (MPT).
///
/// Per the locked design decision: the binary trie of an empty state has root `[0u8; 32]`.
/// Never conflate with the MPT empty root.
pub const EMPTY_BINARY_ROOT: H256 = H256([0u8; 32]);

/// Framing tag for real values in the binary trie layer cache.
///
/// A cached entry is framed as `[CACHE_VALUE_TAG, ...value_bytes]` to
/// distinguish it from a tombstone. This avoids using an empty `Vec<u8>`
/// as a sentinel, which would be ambiguous.
pub const CACHE_VALUE_TAG: u8 = 0x00;

/// Framing tag for tombstone entries in the binary trie layer cache.
///
/// A tombstone entry is exactly `[CACHE_TOMBSTONE_TAG]` (a single byte).
/// It records that a leaf was deleted, so that reads do not fall through
/// to the MPT base in `TransitionBackend`.
pub const CACHE_TOMBSTONE_TAG: u8 = 0x01;

/// BLAKE3 hash of arbitrary input, returns 32 bytes.
pub fn blake3_hash(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_hash_produces_32_bytes() {
        let result = blake3_hash(b"hello");
        assert_eq!(result.len(), 32);
    }

    #[test]
    fn blake3_hash_is_deterministic() {
        let a = blake3_hash(b"test data");
        let b = blake3_hash(b"test data");
        assert_eq!(a, b);
    }

    #[test]
    fn blake3_hash_different_inputs_differ() {
        let a = blake3_hash(b"foo");
        let b = blake3_hash(b"bar");
        assert_ne!(a, b);
    }
}
