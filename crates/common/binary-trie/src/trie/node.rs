//! Node hashing: the leaf and branch preimages that commit the
//! tree's contents to a single BLAKE3 root.

use ethereum_types::H256;

use super::bits::encode_bit_prefix;

/// Root hash of an empty tree: a 32-zero-byte sentinel, not a hash output.
pub const EMPTY_TRIE_ROOT: H256 = H256([0u8; 32]);

const LEAF_NODE_TAG: u8 = 0x00;
const BRANCH_NODE_TAG: u8 = 0x01;

pub(crate) fn blake3_hash(data: &[u8]) -> H256 {
    H256(*blake3::hash(data).as_bytes())
}

/// Hash committing to a leaf: `blake3(0x00 ‖ full_key ‖ value)`.
/// The complete key is committed so a leaf's meaning never depends
/// on the path taken to reach it.
pub(super) fn leaf_hash(key: &[u8], value: &[u8; 32]) -> H256 {
    let mut preimage = Vec::with_capacity(1 + key.len() + 32);
    preimage.push(LEAF_NODE_TAG);
    preimage.extend_from_slice(key);
    preimage.extend_from_slice(value);
    blake3_hash(&preimage)
}

/// Hash committing to a branch:
/// `blake3(0x01 ‖ encode_bit_prefix(prefix) ‖ left ‖ right)`.
pub(super) fn branch_hash(prefix: &[u8], left: H256, right: H256) -> H256 {
    let encoded_prefix = encode_bit_prefix(prefix);
    let mut preimage = Vec::with_capacity(1 + encoded_prefix.len() + 64);
    preimage.push(BRANCH_NODE_TAG);
    preimage.extend_from_slice(&encoded_prefix);
    preimage.extend_from_slice(left.as_bytes());
    preimage.extend_from_slice(right.as_bytes());
    blake3_hash(&preimage)
}
