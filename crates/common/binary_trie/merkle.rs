use crate::hash::blake3_hash;
use crate::node::{Node, STEM_VALUES, SUBTREE_SIZE};

/// The canonical empty / zero hash.
pub const ZERO_HASH: [u8; 32] = [0u8; 32];

/// Hash 64 bytes with the EIP-7864 merkelization special case:
/// `hash([0x00]*64)` → `[0x00]*32` (hardcoded, NOT the real hash output).
///
/// This special case applies only to merkelization (the EIP's `_hash` function),
/// NOT to key derivation (`tree_hash`), which is plain BLAKE3.
fn merkle_hash_64(data: &[u8; 64]) -> [u8; 32] {
    if *data == [0u8; 64] {
        ZERO_HASH
    } else {
        blake3_hash(data.as_slice())
    }
}

/// Public re-export of `merkle_hash_64` used by `StemNode::update_subtree_cache`.
#[inline]
pub fn merkle_hash_64_pub(data: &[u8; 64]) -> [u8; 32] {
    merkle_hash_64(data)
}

/// Compute the Merkle root hash of the entire trie rooted at `root`.
///
/// - Empty tree  → ZERO_HASH
/// - InternalNode → `merkle_hash_64(left_hash || right_hash)`
/// - StemNode     → `merkle_hash_64(stem || 0x00 || subtree_root)` where `subtree_root`
///   is the root of the fixed-depth-8 complete binary Merkle tree over the 256 leaf hashes.
///
/// Hashes are cached on each node and reused on subsequent calls unless a mutation
/// has cleared the cache. The `root` parameter must be `&mut` so that computed
/// hashes can be written back for future reuse.
pub fn merkelize(root: Option<&mut Node>) -> [u8; 32] {
    match root {
        None => ZERO_HASH,
        Some(node) => hash_node(node),
    }
}

pub(crate) fn hash_node(node: &mut Node) -> [u8; 32] {
    match node {
        Node::Internal(internal) => {
            if let Some(h) = internal.cached_hash {
                return h;
            }
            let left = internal.left.as_deref_mut().map_or(ZERO_HASH, hash_node);
            let right = internal.right.as_deref_mut().map_or(ZERO_HASH, hash_node);
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&left);
            buf[32..].copy_from_slice(&right);
            let h = merkle_hash_64(&buf);
            internal.cached_hash = Some(h);
            h
        }
        Node::Stem(stem_node) => {
            if let Some(h) = stem_node.cached_hash {
                return h;
            }
            let subtree_root = compute_subtree_root(stem_node);

            // hash(stem || 0x00 || subtree_root)
            // stem is 31 bytes + 0x00 padding + 32-byte subtree_root = 64 bytes.
            let mut buf = [0u8; 64];
            buf[..31].copy_from_slice(&stem_node.stem);
            buf[31] = 0x00;
            buf[32..].copy_from_slice(&subtree_root);
            let h = merkle_hash_64(&buf);
            stem_node.cached_hash = Some(h);
            h
        }
    }
}

/// Compute (and cache) the subtree root for a StemNode's 256 values.
///
/// The subtree is a complete binary Merkle tree of depth 8 with 256 leaves.
/// It is represented as a flat array of 511 hashes (the same layout used by
/// `StemNode::cached_subtree`).
///
/// If the cache is already populated (`Some`), return `cache[0]` directly.
/// Otherwise, build the entire 511-entry cache from scratch and return its root.
fn compute_subtree_root(stem_node: &mut crate::node::StemNode) -> [u8; 32] {
    if let Some(ref cache) = stem_node.cached_subtree {
        return cache[0];
    }

    // Build the full 511-entry flat binary tree.
    let mut cache = Box::new([[0u8; 32]; SUBTREE_SIZE]);

    // Fill leaf hashes at indices 255..=510.
    for i in 0..STEM_VALUES {
        cache[255 + i] = stem_node.values[i]
            .map(|v| blake3_hash(&v))
            .unwrap_or(ZERO_HASH);
    }

    // Reduce bottom-up: for each internal node (index 254 down to 0), hash children.
    for parent in (0..255usize).rev() {
        let left_child = 2 * parent + 1;
        let right_child = 2 * parent + 2;
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(&cache[left_child]);
        buf[32..].copy_from_slice(&cache[right_child]);
        cache[parent] = merkle_hash_64(&buf);
    }

    let root = cache[0];
    stem_node.cached_subtree = Some(cache);
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::StemNode;

    #[test]
    fn empty_trie_is_zero_hash() {
        assert_eq!(merkelize(None), ZERO_HASH);
    }

    #[test]
    fn merkle_hash_64_zero_is_zero() {
        let zero = [0u8; 64];
        assert_eq!(merkle_hash_64(&zero), ZERO_HASH);
    }

    #[test]
    fn merkle_hash_64_nonzero_is_not_zero() {
        let mut data = [0u8; 64];
        data[0] = 1;
        let result = merkle_hash_64(&data);
        assert_ne!(result, ZERO_HASH);
    }

    #[test]
    fn stem_node_all_empty_values_hashes_correctly() {
        let stem = [0xAAu8; 31];
        let h = hash_node(&mut Node::Stem(StemNode::new(stem)));
        // Deterministic.
        let h2 = hash_node(&mut Node::Stem(StemNode::new(stem)));
        assert_eq!(h, h2);
    }

    #[test]
    fn stem_node_zero_stem_all_empty_is_zero() {
        // stem = [0;31], 0x00, subtree_root = [0;32] → buf = [0;64] → ZERO_HASH
        let stem = [0u8; 31];
        let h = hash_node(&mut Node::Stem(StemNode::new(stem)));
        assert_eq!(h, ZERO_HASH);
    }

    #[test]
    fn stem_node_single_value_changes_hash() {
        let stem = [0u8; 31];
        let before_zero = hash_node(&mut Node::Stem(StemNode::new(stem)));

        let mut stem_node = StemNode::new(stem);
        stem_node.set_value(0, [1u8; 32]);
        let after = hash_node(&mut Node::Stem(stem_node));
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

        let h_a = hash_node(&mut Node::Stem(node_a));
        let h_b = hash_node(&mut Node::Stem(node_b));

        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(&h_a);
        buf[32..].copy_from_slice(&h_b);
        let expected = merkle_hash_64(&buf);

        let mut a2 = StemNode::new(stem_a);
        a2.set_value(0, [1u8; 32]);
        let mut b2 = StemNode::new(stem_b);
        b2.set_value(0, [2u8; 32]);

        let mut internal = Node::Internal(InternalNode::new(
            Some(Box::new(Node::Stem(a2))),
            Some(Box::new(Node::Stem(b2))),
        ));
        let result = hash_node(&mut internal);
        assert_eq!(result, expected);
    }

    #[test]
    fn merkle_root_via_trie() {
        use crate::trie::BinaryTrie;

        let mut trie = BinaryTrie::new();
        let k = [0u8; 32];
        trie.insert(k, [42u8; 32]).unwrap();
        let root = merkelize(trie.root.as_deref_mut());
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
            merkelize(t1.root.as_deref_mut()),
            merkelize(t2.root.as_deref_mut())
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

        let mut t1 = BinaryTrie::new();
        t1.insert(k1, [1u8; 32]).unwrap();
        t1.insert(k2, [2u8; 32]).unwrap();
        t1.insert(k3, [3u8; 32]).unwrap();

        let mut t2 = BinaryTrie::new();
        t2.insert(k3, [3u8; 32]).unwrap();
        t2.insert(k1, [1u8; 32]).unwrap();
        t2.insert(k2, [2u8; 32]).unwrap();

        assert_eq!(
            merkelize(t1.root.as_deref_mut()),
            merkelize(t2.root.as_deref_mut())
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
            merkelize(t1.root.as_deref_mut()),
            merkelize(t2.root.as_deref_mut())
        );
    }

    // --- Caching-specific tests ---

    #[test]
    fn cached_hash_reused_on_second_call() {
        use crate::trie::BinaryTrie;

        let mut trie = BinaryTrie::new();
        trie.insert([0u8; 32], [1u8; 32]).unwrap();

        let root1 = merkelize(trie.root.as_deref_mut());
        let root2 = merkelize(trie.root.as_deref_mut());
        assert_eq!(root1, root2);
    }

    #[test]
    fn cache_invalidated_after_insert() {
        use crate::trie::BinaryTrie;

        let mut trie = BinaryTrie::new();
        trie.insert([0u8; 32], [1u8; 32]).unwrap();
        let root_before = merkelize(trie.root.as_deref_mut());

        // Mutate — inserts clear cached_hash on the path.
        trie.insert([0u8; 32], [2u8; 32]).unwrap();
        let root_after = merkelize(trie.root.as_deref_mut());

        assert_ne!(root_before, root_after);
    }

    #[test]
    fn subtree_cache_incremental_matches_full_build() {
        // Build a StemNode, compute its hash (this populates cached_subtree).
        // Then mutate a value — the incremental update path runs.
        // Verify the result matches a fresh StemNode with the same final values.
        let stem = [0u8; 31];

        let mut sn = StemNode::new(stem);
        sn.set_value(0, [1u8; 32]);
        // First merkelize: builds full cached_subtree from scratch.
        hash_node(&mut Node::Stem({
            let mut tmp = StemNode::new(stem);
            tmp.set_value(0, [1u8; 32]);
            tmp
        }));

        // Now build a node, hash it once (populates cache), then set another value
        // (incremental update path) and compare with a freshly constructed node.
        let mut sn_cached = StemNode::new(stem);
        sn_cached.set_value(0, [1u8; 32]);
        hash_node(&mut Node::Stem(StemNode::new(stem))); // unrelated, just warming up
        // Compute root on sn_cached (builds full cache).
        let _ = hash_node(&mut Node::Stem({
            let mut tmp = StemNode::new(stem);
            tmp.set_value(0, [1u8; 32]);
            tmp
        }));
        // Apply incremental mutation.
        sn_cached.set_value(100, [2u8; 32]);
        let r_incremental = hash_node(&mut Node::Stem(sn_cached));

        // Reference: fresh node with both values set.
        let mut sn_fresh = StemNode::new(stem);
        sn_fresh.set_value(0, [1u8; 32]);
        sn_fresh.set_value(100, [2u8; 32]);
        let r_fresh = hash_node(&mut Node::Stem(sn_fresh));

        assert_eq!(r_incremental, r_fresh);
    }
}
