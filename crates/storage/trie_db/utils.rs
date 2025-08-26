#[cfg(feature = "libmdbx")]
// In order to use NodeHash as key in a dupsort table we must encode it into a fixed size type
pub fn node_hash_to_fixed_size(node_hash: ethrex_trie::NodeHash) -> [u8; 33] {
    let node_hash_ref = node_hash.as_ref();
    let original_len = node_hash.len();
    // original len will always be lower or equal to 32 bytes
    debug_assert!(original_len <= 32);

    let mut buffer = [0u8; 33];

    // Encode the node as [original_len, node_hash...]
    buffer[0] = original_len as u8;
    buffer[1..(original_len + 1)].copy_from_slice(node_hash_ref);
    buffer
}
