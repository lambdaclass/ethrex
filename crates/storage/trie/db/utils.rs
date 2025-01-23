#[cfg(feature = "libmdbx")]
// In order to use NodeHash as key in a dupsort table we must encode it into a fixed size type
pub fn node_hash_to_fixed_size(node_hash: Vec<u8>) -> [u8; 33] {
    // keep original len so we can re-construct it later
    let original_len = node_hash.len();
    // original len will always be lower or equal to 32 bytes
    debug_assert!(original_len <= 32);
    // Pad the node_hash with zeros to make it fixed_size (in case of inline)
    let mut node_hash = node_hash;
    node_hash.resize(32, 0);
    // Encode the node as [original_len, node_hash...]
    std::array::from_fn(|i| match i {
        0 => original_len as u8,
        n => node_hash[n - 1],
    })
}
