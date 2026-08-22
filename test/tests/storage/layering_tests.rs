use ethrex_common::H256;
use ethrex_storage::apply_prefix_bytes;
use ethrex_trie::Nibbles;

/// Build an `H256` from a single byte, all other bytes zero.
fn h256(b: u8) -> H256 {
    let mut bytes = [0u8; 32];
    bytes[31] = b;
    H256(bytes)
}

/// `apply_prefix_bytes` expands the prefix by hand instead of building a throwaway
/// `Nibbles`. The two must agree byte for byte: this is the key every storage-trie
/// node and flat-KV leaf is stored under, so any drift silently corrupts the whole
/// storage namespace rather than failing loudly.
#[test]
fn apply_prefix_bytes_matches_nibble_expansion() {
    // The expression `apply_prefix_bytes` replaced, kept here as the oracle.
    fn reference(prefix: H256, path: &[u8]) -> Vec<u8> {
        let prefix = Nibbles::from_bytes(prefix.as_bytes());
        let mut key = Vec::with_capacity(prefix.len() + 1 + path.len());
        key.extend_from_slice(prefix.as_ref());
        key.push(17);
        key.extend_from_slice(path);
        key
    }

    // A prefix whose bytes have distinct high and low nibbles, so a swapped or
    // dropped half-byte cannot go unnoticed.
    let mixed = H256(std::array::from_fn(|i| (i as u8).wrapping_mul(37)));
    let cases: [(H256, &[u8]); 5] = [
        (H256::zero(), &[]),
        (h256(0xab), &[0x0, 0xf, 0x7]),
        (H256::repeat_byte(0xa5), &[16]),
        (mixed, &[]),
        (mixed, &[0x1, 0x2, 0x3, 0xe, 0xd, 16]),
    ];

    for (prefix, path) in cases {
        let key = apply_prefix_bytes(prefix, path);
        assert_eq!(key, reference(prefix, path));
        // Layout spelled out, so a change to `Nibbles::from_bytes` that moved both
        // sides in step would still be caught.
        assert_eq!(key.len(), 66 + path.len());
        assert_eq!(&key[..2], &[prefix[0] >> 4, prefix[0] & 0x0f]);
        assert_eq!(key[64], 16, "leaf flag");
        assert_eq!(key[65], 17, "storage-trie separator");
        assert_eq!(&key[66..], path);
    }
}
