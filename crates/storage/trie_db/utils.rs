#[cfg(feature = "libmdbx")]
// In order to use NodeHash as key in a dupsort table we must encode it into a fixed size type
pub fn nibbles_to_fixed_size(nibbles: ethrex_trie::Nibbles) -> [u8; 33] {
    let node_hash_ref = nibbles.to_bytes();
    let original_len = node_hash_ref.len();

    let mut buffer = [0u8; 33];

    // Encode the node as [original_len, node_hash...]
    buffer[32] = nibbles.len() as u8;
    buffer[..original_len].copy_from_slice(&node_hash_ref);
    buffer
}
