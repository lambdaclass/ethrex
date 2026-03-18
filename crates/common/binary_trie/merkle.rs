use crate::hash::TreeHasher;
use crate::node::{Node, STEM_VALUES};

/// The canonical empty / zero hash.
pub const ZERO_HASH: [u8; 32] = [0u8; 32];

/// Hash 64 bytes with the EIP-7864 special case:
/// `hash([0x00]*64)` → `[0x00]*32` (hardcoded, NOT the real hash output).
pub fn hash_64<H: TreeHasher>(data: &[u8; 64]) -> [u8; 32] {
    if *data == [0u8; 64] {
        ZERO_HASH
    } else {
        H::hash(data.as_slice())
    }
}

/// Compute the Merkle root hash of the entire trie rooted at `root`.
///
/// - Empty tree  → ZERO_HASH
/// - InternalNode → `hash_64(left_hash || right_hash)`
/// - StemNode     → `hash_64(stem || 0x00 || subtree_root)` where `subtree_root`
///   is the root of the fixed-depth-8 complete binary Merkle tree over the 256 leaf hashes.
pub fn merkelize<H: TreeHasher>(root: Option<&Node>) -> [u8; 32] {
    match root {
        None => ZERO_HASH,
        Some(node) => hash_node::<H>(node),
    }
}

fn hash_node<H: TreeHasher>(node: &Node) -> [u8; 32] {
    match node {
        Node::Internal(internal) => {
            let left = internal.left.as_deref().map_or(ZERO_HASH, hash_node::<H>);
            let right = internal.right.as_deref().map_or(ZERO_HASH, hash_node::<H>);
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&left);
            buf[32..].copy_from_slice(&right);
            hash_64::<H>(&buf)
        }
        Node::Stem(stem_node) => {
            // Step 1: compute the 256 leaf hashes.
            let mut level: Vec<[u8; 32]> = (0..STEM_VALUES)
                .map(|i| {
                    stem_node.values[i]
                        .map(|v| H::hash(&v))
                        .unwrap_or(ZERO_HASH)
                })
                .collect();

            // Step 2: reduce the 256 leaves pairwise 8 times → single root.
            for _ in 0..8 {
                let next_len = level.len() / 2;
                let mut next = Vec::with_capacity(next_len);
                for pair in level.chunks_exact(2) {
                    let mut buf = [0u8; 64];
                    buf[..32].copy_from_slice(&pair[0]);
                    buf[32..].copy_from_slice(&pair[1]);
                    next.push(hash_64::<H>(&buf));
                }
                level = next;
            }
            let subtree_root = level[0];

            // Step 3: hash(stem || 0x00 || subtree_root) = hash(32+1+31 = 64 bytes? No.)
            // The stem is 31 bytes; 0x00 padding byte; subtree_root is 32 bytes.
            // Total: 31 + 1 + 32 = 64 bytes.
            let mut buf = [0u8; 64];
            buf[..31].copy_from_slice(&stem_node.stem);
            buf[31] = 0x00;
            buf[32..].copy_from_slice(&subtree_root);
            hash_64::<H>(&buf)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::Blake3Hasher;
    use crate::node::StemNode;

    #[test]
    fn empty_trie_is_zero_hash() {
        assert_eq!(merkelize::<Blake3Hasher>(None), ZERO_HASH);
    }

    #[test]
    fn hash_64_zero_is_zero() {
        let zero = [0u8; 64];
        assert_eq!(hash_64::<Blake3Hasher>(&zero), ZERO_HASH);
    }

    #[test]
    fn hash_64_nonzero_is_not_zero() {
        let mut data = [0u8; 64];
        data[0] = 1;
        let result = hash_64::<Blake3Hasher>(&data);
        assert_ne!(result, ZERO_HASH);
    }

    #[test]
    fn stem_node_all_empty_values_hashes_correctly() {
        // An all-empty StemNode should produce a deterministic non-zero hash
        // (because the stem itself is non-zero and hashing stem||0x00||zero_subtree
        // is not the zero-input special case unless stem is also zero).
        let stem = [0xAAu8; 31];
        let stem_node = StemNode::new(stem);
        let h = hash_node::<Blake3Hasher>(&Node::Stem(stem_node));
        // Just verify it is deterministic.
        let stem_node2 = StemNode::new(stem);
        let h2 = hash_node::<Blake3Hasher>(&Node::Stem(stem_node2));
        assert_eq!(h, h2);
    }

    #[test]
    fn stem_node_zero_stem_all_empty_is_zero() {
        // stem = [0;31], 0x00, subtree_root = [0;32] → buf = [0;64] → ZERO_HASH
        let stem = [0u8; 31];
        let stem_node = StemNode::new(stem);
        let h = hash_node::<Blake3Hasher>(&Node::Stem(stem_node));
        assert_eq!(h, ZERO_HASH);
    }

    #[test]
    fn stem_node_single_value_changes_hash() {
        let stem = [0u8; 31];
        let mut stem_node = StemNode::new(stem);
        let before_zero = hash_node::<Blake3Hasher>(&Node::Stem(StemNode::new(stem)));

        stem_node.set_value(0, [1u8; 32]);
        let after = hash_node::<Blake3Hasher>(&Node::Stem(stem_node));
        // Setting a value must change the hash.
        assert_ne!(before_zero, after);
    }

    #[test]
    fn internal_node_hash_uses_children() {
        use crate::node::InternalNode;

        let stem_a = [0u8; 31];
        let stem_b = [0xFFu8; 31];
        let mut node_a = StemNode::new(stem_a);
        node_a.set_value(0, [1u8; 32]);
        let mut node_b = StemNode::new(stem_b);
        node_b.set_value(0, [2u8; 32]);

        let h_a = hash_node::<Blake3Hasher>(&Node::Stem(node_a));
        let h_b = hash_node::<Blake3Hasher>(&Node::Stem(node_b));

        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(&h_a);
        buf[32..].copy_from_slice(&h_b);
        let expected = hash_64::<Blake3Hasher>(&buf);

        let mut a2 = StemNode::new(stem_a);
        a2.set_value(0, [1u8; 32]);
        let mut b2 = StemNode::new(stem_b);
        b2.set_value(0, [2u8; 32]);

        let internal = Node::Internal(InternalNode::new(
            Some(Box::new(Node::Stem(a2))),
            Some(Box::new(Node::Stem(b2))),
        ));
        let result = hash_node::<Blake3Hasher>(&internal);
        assert_eq!(result, expected);
    }

    #[test]
    fn merkle_root_via_trie() {
        use crate::trie::BinaryTrie;

        let mut trie = BinaryTrie::new();
        // Insert a single value and verify the root changes from zero.
        let k = [0u8; 32];
        trie.insert(k, [42u8; 32]).unwrap();
        let root = merkelize::<Blake3Hasher>(trie.root.as_deref());
        assert_ne!(root, ZERO_HASH);
    }

    #[test]
    fn merkle_root_is_deterministic() {
        use crate::trie::BinaryTrie;

        let mut t1 = BinaryTrie::new();
        let mut t2 = BinaryTrie::new();
        let k1 = [0u8; 32];
        let mut k2 = [0u8; 32];
        k2[0] = 0x80;

        t1.insert(k1, [1u8; 32]).unwrap();
        t1.insert(k2, [2u8; 32]).unwrap();

        t2.insert(k1, [1u8; 32]).unwrap();
        t2.insert(k2, [2u8; 32]).unwrap();

        assert_eq!(
            merkelize::<Blake3Hasher>(t1.root.as_deref()),
            merkelize::<Blake3Hasher>(t2.root.as_deref())
        );
    }

    #[test]
    fn merkle_root_order_independent() {
        use crate::trie::BinaryTrie;

        let mut k1 = [0u8; 32];
        let mut k2 = [0u8; 32];
        let mut k3 = [0u8; 32];
        k1[0] = 0x20;
        k2[0] = 0x80;
        k3[0] = 0xC0;

        // Insert in order 1, 2, 3
        let mut t1 = BinaryTrie::new();
        t1.insert(k1, [1u8; 32]).unwrap();
        t1.insert(k2, [2u8; 32]).unwrap();
        t1.insert(k3, [3u8; 32]).unwrap();

        // Insert in order 3, 1, 2
        let mut t2 = BinaryTrie::new();
        t2.insert(k3, [3u8; 32]).unwrap();
        t2.insert(k1, [1u8; 32]).unwrap();
        t2.insert(k2, [2u8; 32]).unwrap();

        assert_eq!(
            merkelize::<Blake3Hasher>(t1.root.as_deref()),
            merkelize::<Blake3Hasher>(t2.root.as_deref())
        );
    }

    #[test]
    fn merkle_root_different_values_differ() {
        use crate::trie::BinaryTrie;

        let k = [0u8; 32];
        let mut t1 = BinaryTrie::new();
        t1.insert(k, [1u8; 32]).unwrap();

        let mut t2 = BinaryTrie::new();
        t2.insert(k, [2u8; 32]).unwrap();

        assert_ne!(
            merkelize::<Blake3Hasher>(t1.root.as_deref()),
            merkelize::<Blake3Hasher>(t2.root.as_deref())
        );
    }
}
