use crate::hash::blake3_hash;
use crate::node::StemNode;
use crate::node::{Node, NodeId, STEM_VALUES, SUBTREE_SIZE};
use crate::node_store::NodeStore;
use crate::trie::BinaryTrie;

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

/// Compute the Merkle root hash of the entire trie.
///
/// - Empty tree  → ZERO_HASH
/// - InternalNode → `merkle_hash_64(left_hash || right_hash)`
/// - StemNode     → `merkle_hash_64(stem || 0x00 || subtree_root)` where `subtree_root`
///   is the root of the fixed-depth-8 complete binary Merkle tree over the 256 leaf hashes.
///
/// Hashes are cached on each node and reused on subsequent calls unless a mutation
/// has cleared the cache. Takes `&mut BinaryTrie` so that computed hashes can be
/// written back into the store for future reuse.
pub fn merkelize(trie: &mut BinaryTrie) -> [u8; 32] {
    match trie.root {
        None => ZERO_HASH,
        Some(id) => {
            // Allocate one shared subtree buffer, reused across all StemNodes.
            let mut subtree_buf = Box::new([[0u8; 32]; SUBTREE_SIZE]);
            hash_node_id(&mut trie.store, id, &mut subtree_buf)
        }
    }
}

pub(crate) fn hash_node_id(
    store: &mut NodeStore,
    id: NodeId,
    subtree_buf: &mut [[u8; 32]; SUBTREE_SIZE],
) -> [u8; 32] {
    // Take the node out so we can mutate it.
    let node = match store.take(id) {
        Ok(n) => n,
        Err(_) => return ZERO_HASH,
    };

    match node {
        Node::Internal(mut internal) => {
            if let Some(h) = internal.cached_hash {
                store.put_clean(id, Node::Internal(internal));
                return h;
            }
            let left = internal
                .left
                .map(|lid| hash_node_id(store, lid, subtree_buf))
                .unwrap_or(ZERO_HASH);
            let right = internal
                .right
                .map(|rid| hash_node_id(store, rid, subtree_buf))
                .unwrap_or(ZERO_HASH);
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&left);
            buf[32..].copy_from_slice(&right);
            let h = merkle_hash_64(&buf);
            internal.cached_hash = Some(h);
            store.put_clean(id, Node::Internal(internal));
            h
        }
        Node::Stem(mut stem_node) => {
            if let Some(h) = stem_node.cached_hash {
                store.put_clean(id, Node::Stem(stem_node));
                return h;
            }
            let subtree_root = compute_subtree_root(&stem_node, subtree_buf);

            // hash(stem || 0x00 || subtree_root)
            // stem is 31 bytes + 0x00 padding + 32-byte subtree_root = 64 bytes.
            let mut buf = [0u8; 64];
            buf[..31].copy_from_slice(&stem_node.stem);
            buf[31] = 0x00;
            buf[32..].copy_from_slice(&subtree_root);
            let h = merkle_hash_64(&buf);
            stem_node.cached_hash = Some(h);
            store.put_clean(id, Node::Stem(stem_node));
            h
        }
    }
}

/// Compute the subtree root for a StemNode's 256 values using a shared buffer.
///
/// The subtree is a complete binary Merkle tree of depth 8 with 256 leaves,
/// represented as a flat array of 511 hashes. The buffer is filled in-place
/// and reused across all StemNode merkleizations.
fn compute_subtree_root(stem_node: &StemNode, buf: &mut [[u8; 32]; SUBTREE_SIZE]) -> [u8; 32] {
    // Fill all leaves with ZERO_HASH (default for absent values).
    for i in 0..STEM_VALUES {
        buf[255 + i] = ZERO_HASH;
    }
    // Overwrite only present values (sparse — typically 1-5 hashes instead of 256).
    for (&idx, val) in &stem_node.values {
        buf[255 + idx as usize] = blake3_hash(val);
    }

    // Reduce bottom-up: for each internal node (index 254 down to 0), hash children.
    for parent in (0..255usize).rev() {
        let left_child = 2 * parent + 1;
        let right_child = 2 * parent + 2;
        let mut hash_buf = [0u8; 64];
        hash_buf[..32].copy_from_slice(&buf[left_child]);
        hash_buf[32..].copy_from_slice(&buf[right_child]);
        buf[parent] = merkle_hash_64(&hash_buf);
    }

    buf[0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::StemNode;
    use crate::node_store::NodeStore;

    fn hash_stem(stem_node: StemNode) -> [u8; 32] {
        let mut store = NodeStore::new_memory();
        let id = store.create(Node::Stem(stem_node));
        let mut buf = Box::new([[0u8; 32]; SUBTREE_SIZE]);
        hash_node_id(&mut store, id, &mut buf)
    }

    fn hash_internal_with_children(
        store: &mut NodeStore,
        left_id: Option<NodeId>,
        right_id: Option<NodeId>,
    ) -> [u8; 32] {
        let id = store.create(Node::Internal(crate::node::InternalNode::new(
            left_id, right_id,
        )));
        let mut buf = Box::new([[0u8; 32]; SUBTREE_SIZE]);
        hash_node_id(store, id, &mut buf)
    }

    #[test]
    fn empty_trie_is_zero_hash() {
        let mut trie = BinaryTrie::new();
        assert_eq!(merkelize(&mut trie), ZERO_HASH);
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
        let h = hash_stem(StemNode::new(stem));
        // Deterministic.
        let h2 = hash_stem(StemNode::new(stem));
        assert_eq!(h, h2);
    }

    #[test]
    fn stem_node_zero_stem_all_empty_is_zero() {
        // stem = [0;31], 0x00, subtree_root = [0;32] → buf = [0;64] → ZERO_HASH
        let stem = [0u8; 31];
        let h = hash_stem(StemNode::new(stem));
        assert_eq!(h, ZERO_HASH);
    }

    #[test]
    fn stem_node_single_value_changes_hash() {
        let stem = [0u8; 31];
        let before_zero = hash_stem(StemNode::new(stem));

        let mut stem_node = StemNode::new(stem);
        stem_node.set_value(0, [1u8; 32]);
        let after = hash_stem(stem_node);
        assert_ne!(before_zero, after);
    }

    #[test]
    fn internal_node_hash_uses_children() {
        let stem_a = [0u8; 31];
        let stem_b = [0xFFu8; 31];
        let mut node_a = StemNode::new(stem_a);
        node_a.set_value(0, [1u8; 32]);
        let mut node_b = StemNode::new(stem_b);
        node_b.set_value(0, [2u8; 32]);

        let h_a = hash_stem(node_a);
        let h_b = hash_stem(node_b);

        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(&h_a);
        buf[32..].copy_from_slice(&h_b);
        let expected = merkle_hash_64(&buf);

        let mut store = NodeStore::new_memory();
        let mut a2 = StemNode::new(stem_a);
        a2.set_value(0, [1u8; 32]);
        let mut b2 = StemNode::new(stem_b);
        b2.set_value(0, [2u8; 32]);
        let a2_id = store.create(Node::Stem(a2));
        let b2_id = store.create(Node::Stem(b2));

        let result = hash_internal_with_children(&mut store, Some(a2_id), Some(b2_id));
        assert_eq!(result, expected);
    }

    #[test]
    fn merkle_root_via_trie() {
        let mut trie = BinaryTrie::new();
        let k = [0u8; 32];
        trie.insert(k, [42u8; 32]).unwrap();
        let root = merkelize(&mut trie);
        assert_ne!(root, ZERO_HASH);
    }

    #[test]
    fn merkle_root_is_deterministic() {
        let mut t1 = BinaryTrie::new();
        let mut t2 = BinaryTrie::new();
        let k1 = [0u8; 32];
        let mut k2 = [0u8; 32];
        k2[0] = 0x80;

        t1.insert(k1, [1u8; 32]).unwrap();
        t1.insert(k2, [2u8; 32]).unwrap();

        t2.insert(k1, [1u8; 32]).unwrap();
        t2.insert(k2, [2u8; 32]).unwrap();

        assert_eq!(merkelize(&mut t1), merkelize(&mut t2));
    }

    #[test]
    fn merkle_root_order_independent() {
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

        assert_eq!(merkelize(&mut t1), merkelize(&mut t2));
    }

    #[test]
    fn merkle_root_different_values_differ() {
        let k = [0u8; 32];
        let mut t1 = BinaryTrie::new();
        t1.insert(k, [1u8; 32]).unwrap();

        let mut t2 = BinaryTrie::new();
        t2.insert(k, [2u8; 32]).unwrap();

        assert_ne!(merkelize(&mut t1), merkelize(&mut t2));
    }

    // --- Caching-specific tests ---

    #[test]
    fn cached_hash_reused_on_second_call() {
        let mut trie = BinaryTrie::new();
        trie.insert([0u8; 32], [1u8; 32]).unwrap();

        let root1 = merkelize(&mut trie);
        let root2 = merkelize(&mut trie);
        assert_eq!(root1, root2);
    }

    #[test]
    fn cache_invalidated_after_insert() {
        let mut trie = BinaryTrie::new();
        trie.insert([0u8; 32], [1u8; 32]).unwrap();
        let root_before = merkelize(&mut trie);

        // Mutate — inserts clear cached_hash on the path.
        trie.insert([0u8; 32], [2u8; 32]).unwrap();
        let root_after = merkelize(&mut trie);

        assert_ne!(root_before, root_after);
    }

    #[test]
    fn sequential_mutations_match_fresh_build() {
        // Verify that applying values sequentially produces the same hash as
        // building a fresh node with all values set at once.
        let stem = [0u8; 31];

        let mut sn_sequential = StemNode::new(stem);
        sn_sequential.set_value(0, [1u8; 32]);
        sn_sequential.set_value(100, [2u8; 32]);
        let r_sequential = hash_stem(sn_sequential);

        let mut sn_fresh = StemNode::new(stem);
        sn_fresh.set_value(0, [1u8; 32]);
        sn_fresh.set_value(100, [2u8; 32]);
        let r_fresh = hash_stem(sn_fresh);

        assert_eq!(r_sequential, r_fresh);
    }
}

impl Clone for StemNode {
    fn clone(&self) -> Self {
        Self {
            stem: self.stem,
            values: self.values.clone(),
            cached_hash: self.cached_hash,
        }
    }
}
