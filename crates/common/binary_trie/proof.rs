use crate::error::BinaryTrieError;
use crate::hash::blake3_hash;
use crate::merkle::{ZERO_HASH, merkle_hash_64_pub};
use crate::node::{Node, NodeId, SUBTREE_SIZE, StemNode, stem_bit};
use crate::node_store::NodeStore;
use crate::trie::{BinaryTrie, split_key};

/// A Merkle proof for a single key in the binary trie.
///
/// The `siblings` vector contains one hash per level traversed:
/// - The first `stem_depth` entries are sibling hashes from the external tree
///   (one per `InternalNode` traversed from root to the `StemNode`).
/// - The remaining entries depend on `node_hash`:
///   - If `node_hash` is `None` (same-stem or None-child case): the last 8
///     entries are sibling hashes from within the `StemNode`'s depth-8 subtree.
///   - If `node_hash` is `Some` (different-stem case): no subtree siblings are
///     appended; `node_hash` holds the found `StemNode`'s precomputed hash.
///
/// Call `state_root()` on the trie before calling `get_proof()` so that all
/// node hashes are cached; otherwise proof generation returns an error.
pub struct BinaryTrieProof {
    /// Sibling hashes from root to leaf (external tree levels only for
    /// different-stem proofs; external tree + subtree for same-stem proofs).
    pub siblings: Vec<[u8; 32]>,
    /// Depth at which the `StemNode` was found (or path terminated) in the
    /// external tree.
    pub stem_depth: usize,
    /// The leaf value, if present. `None` for absence proofs.
    pub value: Option<[u8; 32]>,
    /// The stem of the `StemNode` found. `None` when the path terminated at an
    /// empty slot.
    pub stem: Option<[u8; 31]>,
    /// For different-stem absence proofs: the cached hash of the encountered
    /// `StemNode`. `None` for same-stem proofs (presence or absent sub-index)
    /// and None-child proofs.
    pub node_hash: Option<[u8; 32]>,
}

impl BinaryTrieProof {
    /// Verify this proof against `root` for `key`.
    ///
    /// Returns `true` if the proof is consistent with the given root hash.
    pub fn verify(&self, root: [u8; 32], key: [u8; 32]) -> bool {
        let (queried_stem, sub_index) = split_key(&key);

        // Compute the hash that represents this trie position.
        let mut current = match self.node_hash {
            Some(h) => {
                // Different-stem case: the proof carries the found StemNode's
                // hash directly. No subtree reconstruction needed.
                // Siblings should contain exactly stem_depth entries.
                if self.siblings.len() != self.stem_depth {
                    return false;
                }
                h
            }
            None => {
                // Same-stem (or None-child) case: reconstruct via subtree.
                // Siblings must have exactly stem_depth + 8 entries.
                if self.siblings.len() != self.stem_depth + 8 {
                    return false;
                }

                match self.stem {
                    None => {
                        // Path terminated at a None child: the hash of this
                        // slot is ZERO_HASH.
                        ZERO_HASH
                    }
                    Some(proof_stem) => {
                        // Same-stem case: reconstruct the StemNode hash.
                        let subtree_siblings = &self.siblings[self.stem_depth..];
                        let leaf_hash = match self.value {
                            Some(ref v) => blake3_hash(v),
                            None => ZERO_HASH,
                        };
                        let subtree_root =
                            reconstruct_subtree_root(leaf_hash, sub_index, subtree_siblings);
                        let mut stem_buf = [0u8; 64];
                        stem_buf[..31].copy_from_slice(&proof_stem);
                        stem_buf[31] = 0x00;
                        stem_buf[32..].copy_from_slice(&subtree_root);
                        merkle_hash_64_pub(&stem_buf)
                    }
                }
            }
        };

        // Reconstruct the external tree root using the external siblings.
        let external_siblings = &self.siblings[..self.stem_depth];
        for (i, sibling) in external_siblings.iter().enumerate().rev() {
            let depth = i;
            let bit = stem_bit(&queried_stem, depth);
            let mut buf = [0u8; 64];
            if bit == 0 {
                // Current node was on the left; sibling is on the right.
                buf[..32].copy_from_slice(&current);
                buf[32..].copy_from_slice(sibling);
            } else {
                // Current node was on the right; sibling is on the left.
                buf[..32].copy_from_slice(sibling);
                buf[32..].copy_from_slice(&current);
            }
            current = merkle_hash_64_pub(&buf);
        }

        current == root
    }
}

/// Reconstruct the subtree root from a leaf hash + 8 sibling hashes.
///
/// `sub_index` is the 0-based leaf index (0-255). `siblings` must have exactly
/// 8 entries, ordered from leaf level up to the subtree root.
fn reconstruct_subtree_root(leaf_hash: [u8; 32], sub_index: u8, siblings: &[[u8; 32]]) -> [u8; 32] {
    debug_assert_eq!(siblings.len(), 8);

    let mut current = leaf_hash;
    let mut idx = sub_index;

    for sibling in siblings.iter() {
        let mut buf = [0u8; 64];
        if idx % 2 == 0 {
            // Even index → left child; sibling is on the right.
            buf[..32].copy_from_slice(&current);
            buf[32..].copy_from_slice(sibling);
        } else {
            // Odd index → right child; sibling is on the left.
            buf[..32].copy_from_slice(sibling);
            buf[32..].copy_from_slice(&current);
        }
        current = merkle_hash_64_pub(&buf);
        idx /= 2;
    }

    current
}

/// Compute the 8 sibling hashes for `sub_index` within the StemNode's
/// 256-value subtree, ordered from the leaf level up to the subtree root.
///
/// The subtree is a flat binary tree of 511 entries:
///   - indices 255..=510 are the 256 leaf hashes
///   - index 0 is the subtree root
///
/// Builds the full subtree from the StemNode's values each time (no cache).
pub(crate) fn subtree_siblings(stem_node: &StemNode, sub_index: u8) -> Vec<[u8; 32]> {
    let cache = build_subtree(stem_node);

    let mut siblings = Vec::with_capacity(8);
    let mut flat_idx = 255 + sub_index as usize; // leaf index in flat tree

    while flat_idx > 0 {
        let sibling_idx = if flat_idx % 2 == 0 {
            flat_idx - 1 // right child → left sibling
        } else {
            flat_idx + 1 // left child → right sibling
        };
        siblings.push(cache[sibling_idx]);
        flat_idx = (flat_idx - 1) / 2; // move to parent
    }

    siblings
}

/// Build the full 511-entry subtree from a StemNode's values (without caching).
fn build_subtree(stem_node: &StemNode) -> Box<[[u8; 32]; SUBTREE_SIZE]> {
    let mut cache = Box::new([[0u8; 32]; SUBTREE_SIZE]);

    for i in 0..256usize {
        cache[255 + i] = ZERO_HASH;
    }
    for (&idx, val) in &stem_node.values {
        cache[255 + idx as usize] = blake3_hash(val);
    }

    for parent in (0..255usize).rev() {
        let left = 2 * parent + 1;
        let right = 2 * parent + 2;
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(&cache[left]);
        buf[32..].copy_from_slice(&cache[right]);
        cache[parent] = merkle_hash_64_pub(&buf);
    }

    cache
}

// ---------------------------------------------------------------------------
// get_proof implementation (free function called from BinaryTrie)
// ---------------------------------------------------------------------------

/// Walk the trie from the root to the key's StemNode, collecting sibling hashes.
///
/// Requires that `state_root()` was called on the trie before this function, so
/// that all `InternalNode::cached_hash` values are populated.
pub(crate) fn get_proof_impl(
    trie: &BinaryTrie,
    key: [u8; 32],
) -> Result<BinaryTrieProof, BinaryTrieError> {
    let (stem, sub_index) = split_key(&key);

    let mut external_siblings: Vec<[u8; 32]> = Vec::new();
    let mut current_id = trie.root;
    let mut depth = 0usize;

    loop {
        match current_id {
            None => {
                // Path terminated at an empty slot — absence proof.
                let subtree_sibs = vec![ZERO_HASH; 8];
                let mut siblings = external_siblings;
                siblings.extend_from_slice(&subtree_sibs);
                return Ok(BinaryTrieProof {
                    siblings,
                    stem_depth: depth,
                    value: None,
                    stem: None,
                    node_hash: None,
                });
            }
            Some(id) => {
                let node = trie.store.get(id)?;
                match node {
                    Node::Internal(internal) => {
                        let bit = stem_bit(&stem, depth);
                        let sibling_id = if bit == 0 {
                            internal.right
                        } else {
                            internal.left
                        };
                        let sibling_hash = cached_hash_of(&trie.store, sibling_id)?;
                        external_siblings.push(sibling_hash);

                        current_id = if bit == 0 {
                            internal.left
                        } else {
                            internal.right
                        };
                        depth += 1;
                    }
                    Node::Stem(stem_node) => {
                        let stem_depth = depth;
                        if stem_node.stem == stem {
                            // Matching stem — presence or absent sub-index proof.
                            let value = stem_node.get_value(sub_index);
                            let subtree_sibs = subtree_siblings(&stem_node, sub_index);
                            let mut siblings = external_siblings;
                            siblings.extend_from_slice(&subtree_sibs);
                            return Ok(BinaryTrieProof {
                                siblings,
                                stem_depth,
                                value,
                                stem: Some(stem_node.stem),
                                node_hash: None,
                            });
                        } else {
                            // Different stem — absence proof.
                            // Carry the found StemNode's cached hash so the
                            // verifier can reconstruct the root without knowing
                            // the StemNode's actual values.
                            let found_hash = stem_node
                                .cached_hash
                                .ok_or(BinaryTrieError::ProofRequiresMerkelization)?;
                            return Ok(BinaryTrieProof {
                                siblings: external_siblings,
                                stem_depth,
                                value: None,
                                stem: Some(stem_node.stem),
                                node_hash: Some(found_hash),
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Return the cached hash of `node_id`, or `ZERO_HASH` for `None`.
/// Returns an error if the node exists but has no cached hash.
fn cached_hash_of(store: &NodeStore, node_id: Option<NodeId>) -> Result<[u8; 32], BinaryTrieError> {
    match node_id {
        None => Ok(ZERO_HASH),
        Some(id) => {
            let node = store.get(id)?;
            match node {
                Node::Internal(internal) => internal
                    .cached_hash
                    .ok_or(BinaryTrieError::ProofRequiresMerkelization),
                Node::Stem(stem_node) => stem_node
                    .cached_hash
                    .ok_or(BinaryTrieError::ProofRequiresMerkelization),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merkle::merkelize;
    use crate::trie::BinaryTrie;

    fn key(stem_byte: u8, sub: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = stem_byte;
        k[31] = sub;
        k
    }

    fn val(b: u8) -> [u8; 32] {
        [b; 32]
    }

    #[test]
    fn proof_existing_key_verifies() {
        let mut trie = BinaryTrie::new();
        let k = key(0xAA, 5);
        trie.insert(k, val(42)).unwrap();
        let root = merkelize(&mut trie);

        let proof = trie.get_proof(k).unwrap();
        assert!(
            proof.verify(root, k),
            "proof for existing key should verify"
        );
    }

    #[test]
    fn proof_absent_key_wrong_stem_verifies() {
        let mut trie = BinaryTrie::new();
        trie.insert(key(0xAA, 0), val(1)).unwrap();
        let root = merkelize(&mut trie);

        // 0xAA and 0xAB share 7 leading bits (differ at bit 7).
        // The trie has only one StemNode (0xAA) at the root or shallow depth.
        // Querying 0xAB will encounter the 0xAA StemNode (wrong stem).
        let absent = key(0xAB, 0);
        let proof = trie.get_proof(absent).unwrap();
        assert!(
            proof.value.is_none(),
            "absent key should produce absence proof"
        );
        assert!(
            proof.verify(root, absent),
            "absence proof (wrong stem) should verify"
        );
    }

    #[test]
    fn proof_absent_key_none_child_verifies() {
        let mut trie = BinaryTrie::new();
        // Insert with MSB=1 (stem starts 0x80).
        trie.insert(key(0x80, 0), val(1)).unwrap();
        let root = merkelize(&mut trie);

        // Query MSB=0 — routes to the left child which is None.
        let absent = key(0x00, 0);
        let proof = trie.get_proof(absent).unwrap();
        assert!(proof.value.is_none());
        assert!(
            proof.verify(root, absent),
            "absence proof (None child) should verify"
        );
    }

    #[test]
    fn proof_after_insert_matches_root() {
        let mut trie = BinaryTrie::new();
        for i in 0u8..8 {
            let mut k = [0u8; 32];
            k[0] = i * 16;
            k[31] = i;
            trie.insert(k, val(i)).unwrap();
        }
        let root = merkelize(&mut trie);

        for i in 0u8..8 {
            let mut k = [0u8; 32];
            k[0] = i * 16;
            k[31] = i;
            let proof = trie.get_proof(k).unwrap();
            assert!(
                proof.verify(root, k),
                "proof failed for key with stem byte {:#04x}",
                i * 16
            );
        }
    }

    #[test]
    fn proof_multiple_keys_same_stem() {
        let mut trie = BinaryTrie::new();
        for sub in [0u8, 5, 100, 255] {
            trie.insert(key(0xCC, sub), val(sub)).unwrap();
        }
        let root = merkelize(&mut trie);

        for sub in [0u8, 5, 100, 255] {
            let k = key(0xCC, sub);
            let proof = trie.get_proof(k).unwrap();
            assert_eq!(proof.value, Some(val(sub)));
            assert!(proof.verify(root, k), "proof failed for sub_index {sub}");
        }
    }

    #[test]
    fn tampered_proof_fails_verification() {
        let mut trie = BinaryTrie::new();
        let k = key(0xAA, 3);
        trie.insert(k, val(99)).unwrap();
        let root = merkelize(&mut trie);

        let mut proof = trie.get_proof(k).unwrap();
        assert!(proof.verify(root, k));

        // Flip a byte in the first sibling.
        if let Some(sib) = proof.siblings.first_mut() {
            sib[0] ^= 0xFF;
        }
        assert!(
            !proof.verify(root, k),
            "tampered proof should fail verification"
        );
    }

    #[test]
    fn tampered_value_fails_verification() {
        let mut trie = BinaryTrie::new();
        let k = key(0xBB, 7);
        trie.insert(k, val(10)).unwrap();
        let root = merkelize(&mut trie);

        let mut proof = trie.get_proof(k).unwrap();
        assert!(proof.verify(root, k));

        // Tamper with the value.
        if let Some(ref mut v) = proof.value {
            v[0] ^= 0xFF;
        }
        assert!(
            !proof.verify(root, k),
            "tampered value should fail verification"
        );
    }

    #[test]
    fn proof_empty_trie() {
        let mut trie = BinaryTrie::new();
        let root = merkelize(&mut trie);

        let k = key(0x42, 0);
        let proof = trie.get_proof(k).unwrap();
        assert!(proof.value.is_none());
        assert!(
            proof.verify(root, k),
            "absence proof in empty trie should verify"
        );
    }

    #[test]
    fn proof_wrong_root_fails() {
        let mut trie = BinaryTrie::new();
        let k = key(0xAA, 5);
        trie.insert(k, val(42)).unwrap();
        let root = merkelize(&mut trie);

        let proof = trie.get_proof(k).unwrap();
        let wrong_root = [0xFFu8; 32];
        assert!(
            !proof.verify(wrong_root, k),
            "proof should fail against wrong root"
        );
        // Sanity: still passes against correct root.
        assert!(proof.verify(root, k));
    }

    #[test]
    fn proof_single_stem_node_trie() {
        // Trie with one StemNode, no InternalNodes.
        let mut trie = BinaryTrie::new();
        let k = key(0x00, 0);
        trie.insert(k, val(1)).unwrap();
        let root = merkelize(&mut trie);

        let proof = trie.get_proof(k).unwrap();
        assert_eq!(proof.stem_depth, 0, "single StemNode should be at depth 0");
        assert!(proof.verify(root, k));
    }

    #[test]
    fn proof_sub_index_boundaries() {
        let mut trie = BinaryTrie::new();
        // Insert at sub_index 0 and 255 (boundaries of the 256-leaf subtree).
        trie.insert(key(0xEE, 0), val(10)).unwrap();
        trie.insert(key(0xEE, 255), val(20)).unwrap();
        let root = merkelize(&mut trie);

        let proof_0 = trie.get_proof(key(0xEE, 0)).unwrap();
        assert_eq!(proof_0.value, Some(val(10)));
        assert!(proof_0.verify(root, key(0xEE, 0)));

        let proof_255 = trie.get_proof(key(0xEE, 255)).unwrap();
        assert_eq!(proof_255.value, Some(val(20)));
        assert!(proof_255.verify(root, key(0xEE, 255)));
    }

    #[test]
    fn proof_deep_tree_shared_prefix() {
        let mut trie = BinaryTrie::new();
        // Two stems that differ only at bit 15 (byte 1, bit 7).
        // 0x00_00... vs 0x00_01... — share 15 leading bits.
        let mut k1 = [0u8; 32];
        k1[31] = 1;
        let mut k2 = [0u8; 32];
        k2[1] = 0x01; // differs at bit 15
        k2[31] = 2;
        trie.insert(k1, val(1)).unwrap();
        trie.insert(k2, val(2)).unwrap();
        let root = merkelize(&mut trie);

        let proof1 = trie.get_proof(k1).unwrap();
        assert!(
            proof1.stem_depth > 7,
            "should be deep: {}",
            proof1.stem_depth
        );
        assert!(proof1.verify(root, k1));

        let proof2 = trie.get_proof(k2).unwrap();
        assert!(proof2.verify(root, k2));
    }

    #[test]
    fn proof_after_removal() {
        let mut trie = BinaryTrie::new();
        let k = key(0xAA, 5);
        trie.insert(k, val(42)).unwrap();
        merkelize(&mut trie);

        // Remove the key.
        trie.remove(k).unwrap();
        let root = merkelize(&mut trie);

        // Prove absence of the removed key.
        let proof = trie.get_proof(k).unwrap();
        assert!(proof.value.is_none(), "removed key should be absent");
        assert!(proof.verify(root, k));
    }

    #[test]
    fn proof_dense_trie_absent_key() {
        let mut trie = BinaryTrie::new();
        // Insert 64 keys with different stems.
        for i in 0u8..64 {
            let mut k = [0u8; 32];
            k[0] = i * 4; // spread across the key space
            k[31] = 0;
            trie.insert(k, val(i)).unwrap();
        }
        let root = merkelize(&mut trie);

        // Prove a key that doesn't exist.
        let absent = key(0x03, 0); // 0x03 is between 0x00 and 0x04
        let proof = trie.get_proof(absent).unwrap();
        assert!(proof.value.is_none());
        assert!(proof.verify(root, absent));
    }

    #[test]
    fn proof_without_merkelize_fails() {
        let mut trie = BinaryTrie::new();
        trie.insert(key(0x80, 0), val(1)).unwrap();
        trie.insert(key(0x00, 0), val(2)).unwrap();
        // Don't call merkelize — cached hashes are None.

        let result = trie.get_proof(key(0x80, 0));
        assert!(
            result.is_err(),
            "get_proof without merkelize should return error"
        );
    }

    #[test]
    fn proof_absent_sub_index_on_matching_stem() {
        let mut trie = BinaryTrie::new();
        trie.insert(key(0xDD, 0), val(1)).unwrap();
        let root = merkelize(&mut trie);

        // Query sub_index 1 — same stem, value absent.
        let k = key(0xDD, 1);
        let proof = trie.get_proof(k).unwrap();
        assert!(proof.value.is_none());
        assert!(
            proof.verify(root, k),
            "absence proof (missing sub_index) should verify"
        );
    }
}
