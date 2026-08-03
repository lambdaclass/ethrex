//! Node hashing: the leaf and branch preimages that commit the
//! tree's contents to a single BLAKE3 root.

use ethereum_types::H256;

use crate::error::BinaryTrieError;

use super::bits::{decode_bit_prefix, encode_bit_prefix};

/// Root hash of an empty tree: a 32-zero-byte sentinel, not a hash output.
pub const EMPTY_TRIE_ROOT: H256 = H256([0u8; 32]);

const LEAF_NODE_TAG: u8 = 0x00;
const BRANCH_NODE_TAG: u8 = 0x01;

pub(crate) fn blake3_hash(data: &[u8]) -> H256 {
    H256(*blake3::hash(data).as_bytes())
}

/// A node in stored form: a branch's children are the hashes it
/// commits to, not loaded subtrees.
// Unused until the trie loads nodes from a store; see the note on
// `decode_bit_prefix`.
#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
pub(super) enum StoredNode {
    Leaf {
        key: Vec<u8>,
        value: [u8; 32],
    },
    Branch {
        prefix: Vec<u8>,
        left: H256,
        right: H256,
    },
}

/// Encode a leaf: `0x00 ‖ full_key ‖ value`.
///
/// The complete key is committed, so a leaf's meaning never depends on
/// the path taken to reach it. Self-delimiting on decode because the
/// value is a fixed 32 bytes: whatever lies between the tag and the
/// last 32 bytes is the key.
pub(super) fn encode_leaf(key: &[u8], value: &[u8; 32]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(1 + key.len() + 32);
    encoded.push(LEAF_NODE_TAG);
    encoded.extend_from_slice(key);
    encoded.extend_from_slice(value);
    encoded
}

/// Encode a branch: `0x01 ‖ encode_bit_prefix(prefix) ‖ left ‖ right`.
///
/// Self-delimiting because the prefix encoding carries its own bit
/// count and both children are a fixed 32 bytes.
pub(super) fn encode_branch(prefix: &[u8], left: H256, right: H256) -> Vec<u8> {
    let encoded_prefix = encode_bit_prefix(prefix);
    let mut encoded = Vec::with_capacity(1 + encoded_prefix.len() + 64);
    encoded.push(BRANCH_NODE_TAG);
    encoded.extend_from_slice(&encoded_prefix);
    encoded.extend_from_slice(left.as_bytes());
    encoded.extend_from_slice(right.as_bytes());
    encoded
}

/// Decode a node from its stored bytes.
#[allow(dead_code)]
pub(super) fn decode(encoded: &[u8]) -> Result<StoredNode, BinaryTrieError> {
    match encoded.split_first() {
        Some((&LEAF_NODE_TAG, rest)) => {
            // A key of at least one byte must precede the value: the
            // empty key is a prefix of every other key, so the tree
            // never stores one.
            let key_len = rest
                .len()
                .checked_sub(32)
                .filter(|len| *len > 0)
                .ok_or(BinaryTrieError::MalformedNode("leaf shorter than a value"))?;
            let (key, value) = rest.split_at(key_len);
            Ok(StoredNode::Leaf {
                key: key.to_vec(),
                value: value.try_into().expect("split at len - 32"),
            })
        }
        Some((&BRANCH_NODE_TAG, rest)) => {
            let (prefix, consumed) = decode_bit_prefix(rest)?;
            let children = rest
                .get(consumed..)
                .filter(|children| children.len() == 64)
                .ok_or(BinaryTrieError::MalformedNode("branch children truncated"))?;
            Ok(StoredNode::Branch {
                prefix,
                left: H256::from_slice(&children[..32]),
                right: H256::from_slice(&children[32..]),
            })
        }
        Some(_) => Err(BinaryTrieError::MalformedNode("unknown node tag")),
        None => Err(BinaryTrieError::MalformedNode("empty")),
    }
}

/// Hash committing to a leaf.
pub(super) fn leaf_hash(key: &[u8], value: &[u8; 32]) -> H256 {
    blake3_hash(&encode_leaf(key, value))
}

/// Hash committing to a branch.
pub(super) fn branch_hash(prefix: &[u8], left: H256, right: H256) -> H256 {
    blake3_hash(&encode_branch(prefix, left, right))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(b: u8) -> H256 {
        H256([b; 32])
    }

    #[test]
    fn leaf_round_trips() {
        for key in [vec![0xab], vec![0u8; 34], vec![0xff; 66]] {
            let encoded = encode_leaf(&key, &[7u8; 32]);
            assert_eq!(
                decode(&encoded).unwrap(),
                StoredNode::Leaf {
                    key: key.clone(),
                    value: [7u8; 32]
                }
            );
        }
    }

    #[test]
    fn branch_round_trips_across_byte_boundaries() {
        // 0 bits, inside one byte, exactly one byte, crossing into a
        // second, and a long run.
        for len in [0usize, 3, 8, 9, 17, 528] {
            let prefix: Vec<u8> = (0..len).map(|i| (i % 2) as u8).collect();
            let encoded = encode_branch(&prefix, h(1), h(2));
            assert_eq!(
                decode(&encoded).unwrap(),
                StoredNode::Branch {
                    prefix,
                    left: h(1),
                    right: h(2)
                },
                "prefix of {len} bits"
            );
        }
    }

    #[test]
    fn hashing_is_blake3_of_the_encoding() {
        // The stored form and the hashing preimage are the same bytes,
        // so the two can never drift apart.
        assert_eq!(
            leaf_hash(&[0xab], &[1u8; 32]),
            blake3_hash(&encode_leaf(&[0xab], &[1u8; 32]))
        );
        assert_eq!(
            branch_hash(&[1, 0, 1], h(3), h(4)),
            blake3_hash(&encode_branch(&[1, 0, 1], h(3), h(4)))
        );
    }

    #[test]
    fn decode_rejects_malformed_input() {
        assert!(decode(&[]).is_err(), "empty input");
        assert!(decode(&[0x02, 0x00]).is_err(), "unknown tag");
        // Leaf with no room for a key between tag and 32-byte value.
        assert!(decode(&[&[0x00][..], &[0u8; 32]].concat()).is_err());
        // Branch truncated mid-child.
        let short = encode_branch(&[1], h(1), h(2));
        assert!(decode(&short[..short.len() - 1]).is_err(), "truncated");
    }

    #[test]
    fn decode_rejects_non_canonical_padding() {
        // Three bits declared, but a padding bit set in the packed byte:
        // the encoder never produces this, and accepting it would give
        // two encodings for one node.
        let mut encoded = encode_branch(&[1, 0, 1], h(1), h(2));
        encoded[3] |= 0b0000_0001;
        assert!(decode(&encoded).is_err());
    }
}
