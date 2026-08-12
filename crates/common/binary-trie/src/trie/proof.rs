//! Boundary walks: the per-key proof a verifier can check against a root
//! alone, and the per-step data a range verifier needs on top of that.
//!
//! **The format.** A walk is the sequence of stored-node encodings along the
//! path from the root to the terminal, root first. No framing, no hashes, no
//! key material beyond what the nodes already carry — the target key is
//! something the verifier already holds, so nothing on the wire repeats it.
//!
//! It verifies because a node's stored bytes *are* its hashing preimage (see
//! [`hash_stored_node`] and `node.rs`): each node is hashed and checked against
//! the child pointer that named it, and the first against the root. So the
//! whole proof reduces to a hash chain plus the structural rules of the
//! descent, and there is no second serialization that could drift from the
//! consensus one.
//!
//! The prover lives on [`BinaryTrie`] rather than here — it is the same
//! descent [`BinaryTrie::get`] makes and needs the trie's private node
//! machinery — while everything a *verifier* runs is standalone in this
//! module, which is the half a syncing client compiles against.
//!
//! [`hash_stored_node`]: super::hash_stored_node
//! [`BinaryTrie`]: super::BinaryTrie
//! [`BinaryTrie::get`]: super::BinaryTrie::get

use ethereum_types::H256;
use thiserror::Error;

use super::bits::bytes_to_bits;
use super::hash_stored_node;
use super::node::{EMPTY_TRIE_ROOT, StoredNode, decode};

/// Why a walk did not verify.
///
/// Every variant is a *structural* fault — the proof does not describe a
/// descent through the tree named by `root`. What the walk found once it
/// verified (the wrong leaf, a subtree on the wrong side) is the caller's
/// business, not an error here: an exclusion proof is a perfectly valid walk
/// that ends at somebody else's leaf.
#[derive(Debug, Error, PartialEq, Eq, Clone, Copy)]
pub enum ProofError {
    /// The nodes ran out before the walk reached a terminal.
    #[error("walk proof ends before reaching a terminal node")]
    Truncated,
    /// Nodes follow the terminal. A leaf and a diverging branch both end the
    /// descent, so nothing can legitimately come after one.
    #[error("walk proof carries nodes past its terminal")]
    TrailingNodes,
    /// A node's bytes are not a node.
    #[error("walk proof contains a malformed node")]
    MalformedNode,
    /// A node is not the one the parent (or the root) committed to.
    #[error("walk proof node does not hash to the commitment naming it")]
    HashMismatch,
    /// The empty tree has no nodes, so it admits only the empty walk.
    #[error("the empty root admits only an empty walk proof")]
    EmptyRootConflict,
}

/// One descended branch of a verified walk.
///
/// The sibling is the half of the branch the walk did *not* enter, which is
/// the whole point of recording steps: it is a commitment to a subtree that
/// lies entirely on one side of the walked key, and a range proof is built out
/// of exactly those.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkStep {
    /// Absolute bit index of the branch's split: the path bits consumed to
    /// reach it plus its own prefix length. The bit at this index is the one
    /// the branch decides on.
    pub split: usize,
    /// The key's bit at `split` — the child that was descended into.
    pub taken: u8,
    /// Commitment of the child that was *not* descended into.
    pub sibling: H256,
}

/// Where a verified walk ended.
///
/// Owned rather than borrowed from the proof buffer: a terminal is one node
/// per walk, the copy is a 34- or 66-byte key, and the alternative is a
/// lifetime on every caller plus an invariant re-check to turn the decoded
/// lengths back into slices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkEnd {
    /// The walk reached a leaf. Its key may or may not be the target's: an
    /// exclusion proof ends at whichever leaf occupies the position the target
    /// would have taken.
    AtLeaf { key: Vec<u8>, value: [u8; 32] },
    /// The walk ended at a branch whose covered bits the target's bits diverge
    /// from, or run out inside. `subtree_bits` is the branch's full covered
    /// bit-prefix (the path bits down to it, then its own prefix), so the
    /// subtree's position relative to the target is decided by comparing the
    /// two bit strings; `hash` is the commitment to the whole subtree.
    Diverged { subtree_bits: Vec<u8>, hash: H256 },
    /// The tree is empty, so there was nowhere to walk.
    Empty,
}

/// Check a walk proof against `root` and report what it descended through and
/// where it stopped.
///
/// Reads nothing and holds no trie: `proof` plus `root` plus `key` is
/// everything, which is what makes this the half a client can run against a
/// server it does not trust.
///
/// The terminal classification is computed here, never taken from the proof.
/// A prover cannot declare "the walk diverged" at a branch the key actually
/// descends through, because whether it descends is decided by the branch's
/// own prefix — and that prefix is inside the bytes the hash chain pins.
///
/// # Errors
///
/// Every [`ProofError`]; see that type for what each one means.
pub fn verify_walk(
    root: H256,
    key: &[u8],
    proof: &[Vec<u8>],
) -> Result<(Vec<WalkStep>, WalkEnd), ProofError> {
    if root == EMPTY_TRIE_ROOT {
        return if proof.is_empty() {
            Ok((Vec::new(), WalkEnd::Empty))
        } else {
            Err(ProofError::EmptyRootConflict)
        };
    }

    let bits = bytes_to_bits(key);
    let mut steps = Vec::new();
    // The commitment the next node must hash to: the root, then whichever
    // child pointer the descent followed.
    let mut expected = root;
    // Bits consumed to reach the node under inspection, which is therefore
    // its path from the root.
    let mut depth = 0usize;

    for (index, encoded) in proof.iter().enumerate() {
        if hash_stored_node(encoded) != expected {
            return Err(ProofError::HashMismatch);
        }
        let is_last = index + 1 == proof.len();
        match decode(encoded).map_err(|_| ProofError::MalformedNode)? {
            StoredNode::Leaf { key, value } => {
                if !is_last {
                    return Err(ProofError::TrailingNodes);
                }
                return Ok((steps, WalkEnd::AtLeaf { key, value }));
            }
            StoredNode::Branch {
                prefix,
                left,
                right,
            } => {
                let split = depth + prefix.len();
                // The same condition `BinaryTrie::get_at` descends on, and it
                // must stay the same one: a walk that stopped where a lookup
                // would have continued would prove a different tree's shape.
                // Note the ordering — the slice compare is only reached once
                // `split` is known to be inside the key.
                if split >= bits.len() || bits[depth..split] != prefix[..] {
                    if !is_last {
                        return Err(ProofError::TrailingNodes);
                    }
                    let mut subtree_bits = bits[..depth].to_vec();
                    subtree_bits.extend_from_slice(&prefix);
                    return Ok((
                        steps,
                        WalkEnd::Diverged {
                            subtree_bits,
                            hash: expected,
                        },
                    ));
                }
                let taken = bits[split];
                let (descended, sibling) = if taken == 0 {
                    (left, right)
                } else {
                    (right, left)
                };
                steps.push(WalkStep {
                    split,
                    taken,
                    sibling,
                });
                expected = descended;
                depth = split + 1;
            }
        }
    }
    Err(ProofError::Truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trie::BinaryTrie;

    /// Two keys sharing their first byte and diverging at absolute bit 9.
    fn two_leaf_trie() -> (BinaryTrie, H256) {
        let mut trie = BinaryTrie::new_temp();
        trie.insert(vec![0xaa, 0xbb], [1u8; 32]).unwrap();
        trie.insert(vec![0xaa, 0xcc], [2u8; 32]).unwrap();
        let root = trie.commit().unwrap().root;
        (trie, root)
    }

    #[test]
    fn walk_exposes_steps_and_terminal() {
        let (mut trie, root) = two_leaf_trie();

        let proof = trie.prove_walk(&[0xaa, 0xbb]).unwrap();
        let (steps, end) = verify_walk(root, &[0xaa, 0xbb], &proof).unwrap();
        assert_eq!(steps.len(), 1);
        // 0xaa is common; 0xbb = 1011_1011 and 0xcc = 1100_1100 first differ
        // at the second bit of the second byte, so the root branch's prefix is
        // the nine bits 1010_1010_1 and it splits at absolute bit 9. 0xbb has
        // 0 there, so the target is the left child.
        assert_eq!(steps[0].split, 9);
        assert_eq!(steps[0].taken, 0);
        assert_eq!(
            end,
            WalkEnd::AtLeaf {
                key: vec![0xaa, 0xbb],
                value: [1u8; 32]
            }
        );
    }

    #[test]
    fn the_sibling_is_the_child_that_was_not_taken() {
        let (mut trie, root) = two_leaf_trie();
        let (left_steps, _) = verify_walk(
            root,
            &[0xaa, 0xbb],
            &trie.prove_walk(&[0xaa, 0xbb]).unwrap(),
        )
        .unwrap();
        let (right_steps, right_end) = verify_walk(
            root,
            &[0xaa, 0xcc],
            &trie.prove_walk(&[0xaa, 0xcc]).unwrap(),
        )
        .unwrap();

        // Walking the other key takes the other side of the same split, and
        // each walk's sibling is the terminal the other one reached.
        assert_eq!(right_steps[0].split, 9);
        assert_eq!(right_steps[0].taken, 1);
        let WalkEnd::AtLeaf { key, value } = &right_end else {
            panic!("expected a leaf terminal, got {right_end:?}");
        };
        assert_eq!(
            left_steps[0].sibling,
            hash_stored_node(&super::super::node::encode_leaf(key, value))
        );
    }

    #[test]
    fn a_diverging_key_ends_at_the_branch_it_missed() {
        let (mut trie, root) = two_leaf_trie();
        let proof = trie.prove_walk(&[0x11, 0x22]).unwrap();
        let (steps, end) = verify_walk(root, &[0x11, 0x22], &proof).unwrap();
        assert!(steps.is_empty(), "nothing was descended");
        let WalkEnd::Diverged { subtree_bits, hash } = end else {
            panic!("expected a diverging terminal");
        };
        // The root branch's own covered bits, and the root itself.
        assert_eq!(subtree_bits, bytes_to_bits(&[0xaa, 0x80])[..9].to_vec());
        assert_eq!(hash, root);
    }

    #[test]
    fn a_key_shorter_than_the_branch_prefix_diverges_by_exhaustion() {
        let (mut trie, root) = two_leaf_trie();
        // One byte of key against a nine-bit branch prefix: the descent runs
        // out of bits inside the prefix rather than disagreeing with it.
        let proof = trie.prove_walk(&[0xaa]).unwrap();
        let (steps, end) = verify_walk(root, &[0xaa], &proof).unwrap();
        assert!(steps.is_empty());
        let WalkEnd::Diverged { subtree_bits, .. } = end else {
            panic!("expected a diverging terminal");
        };
        assert_eq!(subtree_bits.len(), 9, "the branch's bits, not the key's");

        // The boundary that decides `>=` from `>`: a key ending *exactly* at
        // a branch's split. The prefix matches to the last bit and there is
        // still no bit left to choose a child with, so the walk stops — and
        // a rule written `split > bits.len()` would instead descend and read
        // past the end of the key.
        let mut aligned = BinaryTrie::new_temp();
        aligned.insert(vec![0xaa, 0x00], [1u8; 32]).unwrap();
        aligned.insert(vec![0xaa, 0x80], [2u8; 32]).unwrap();
        let aligned_root = aligned.commit().unwrap().root;
        let proof = aligned.prove_walk(&[0xaa]).unwrap();
        let (steps, end) = verify_walk(aligned_root, &[0xaa], &proof).unwrap();
        assert!(steps.is_empty());
        let WalkEnd::Diverged { subtree_bits, .. } = end else {
            panic!("expected a diverging terminal at the aligned split");
        };
        assert_eq!(subtree_bits, bytes_to_bits(&[0xaa]));
    }

    #[test]
    fn a_divergence_below_the_root_reports_the_whole_covered_prefix() {
        // `subtree_bits` is the path taken to reach the branch *plus* the
        // branch's own prefix. Reporting only the prefix would place the
        // subtree at the root, and every position comparison built on it —
        // which is how a range proof decides a subtree lies before the origin
        // — would then answer for the wrong subtree.
        let mut trie = BinaryTrie::new_temp();
        for key in [vec![0x00, 0x01], vec![0x00, 0x02], vec![0x80, 0x00]] {
            trie.insert(key, [4u8; 32]).unwrap();
        }
        let root = trie.commit().unwrap().root;

        // Descends left at bit 0, then diverges inside the branch over the
        // two 0x00 keys, whose covered bits run to the bit they disagree on.
        let target = [0x00, 0xff];
        let proof = trie.prove_walk(&target).unwrap();
        let (steps, end) = verify_walk(root, &target, &proof).unwrap();
        assert_eq!(steps.len(), 1, "one descent before the divergence");
        assert_eq!(steps[0].split, 0);
        let WalkEnd::Diverged { subtree_bits, .. } = end else {
            panic!("expected a diverging terminal");
        };
        // 0x01 and 0x02 disagree at absolute bit 14, so the branch covers
        // bits 0..14: one taken at the root plus its own thirteen.
        assert_eq!(subtree_bits, bytes_to_bits(&[0x00, 0x01])[..14].to_vec());
    }

    #[test]
    fn a_truncated_proof_is_rejected() {
        let (mut trie, root) = two_leaf_trie();
        let proof = trie.prove_walk(&[0xaa, 0xbb]).unwrap();
        assert_eq!(proof.len(), 2, "root branch then leaf");
        assert_eq!(
            verify_walk(root, &[0xaa, 0xbb], &proof[..1]),
            Err(ProofError::Truncated)
        );
        assert_eq!(
            verify_walk(root, &[0xaa, 0xbb], &[]),
            Err(ProofError::Truncated)
        );
    }

    #[test]
    fn nodes_past_the_terminal_are_rejected() {
        let (mut trie, root) = two_leaf_trie();
        let mut proof = trie.prove_walk(&[0xaa, 0xbb]).unwrap();
        // A leaf ends the walk, so anything after it is trailing — even a
        // node the tree really holds.
        proof.push(proof[0].clone());
        assert_eq!(
            verify_walk(root, &[0xaa, 0xbb], &proof),
            Err(ProofError::TrailingNodes)
        );

        // Same for a diverging branch terminal.
        let mut diverging = trie.prove_walk(&[0x11, 0x22]).unwrap();
        diverging.push(diverging[0].clone());
        assert_eq!(
            verify_walk(root, &[0x11, 0x22], &diverging),
            Err(ProofError::TrailingNodes)
        );
    }

    #[test]
    fn a_tampered_node_breaks_the_hash_chain() {
        let (mut trie, root) = two_leaf_trie();
        let proof = trie.prove_walk(&[0xaa, 0xbb]).unwrap();
        for node in 0..proof.len() {
            for byte in 0..proof[node].len() {
                let mut tampered = proof.clone();
                tampered[node][byte] ^= 1;
                assert!(
                    verify_walk(root, &[0xaa, 0xbb], &tampered).is_err(),
                    "flipping byte {byte} of node {node} was accepted"
                );
            }
        }
    }

    #[test]
    fn a_node_that_is_not_a_node_is_rejected() {
        // A hash chain that checks out over bytes that do not decode: the
        // root is taken to be whatever this garbage hashes to, so the chain
        // itself cannot catch it.
        let garbage = vec![0x7fu8; 40];
        assert_eq!(
            verify_walk(hash_stored_node(&garbage), &[0xaa], &[garbage]),
            Err(ProofError::MalformedNode)
        );
    }

    #[test]
    fn the_empty_root_admits_only_the_empty_proof() {
        assert_eq!(
            verify_walk(EMPTY_TRIE_ROOT, &[0x01], &[]).unwrap(),
            (Vec::new(), WalkEnd::Empty)
        );
        assert_eq!(
            verify_walk(EMPTY_TRIE_ROOT, &[0x01], &[vec![0]]),
            Err(ProofError::EmptyRootConflict)
        );
        // And an empty trie proves nothing.
        assert!(
            BinaryTrie::new_temp()
                .prove_walk(&[0x01])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_single_leaf_trie_proves_by_one_node() {
        let mut trie = BinaryTrie::new_temp();
        trie.insert(vec![0xaa, 0xbb], [3u8; 32]).unwrap();
        let root = trie.commit().unwrap().root;

        // Both the key itself and a key that is absent reach the same leaf:
        // the second is the exclusion proof.
        for key in [vec![0xaa, 0xbb], vec![0x00, 0x01], vec![0xff, 0xff]] {
            let proof = trie.prove_walk(&key).unwrap();
            assert_eq!(proof.len(), 1);
            let (steps, end) = verify_walk(root, &key, &proof).unwrap();
            assert!(steps.is_empty());
            assert_eq!(
                end,
                WalkEnd::AtLeaf {
                    key: vec![0xaa, 0xbb],
                    value: [3u8; 32]
                }
            );
        }
    }

    #[test]
    fn a_proof_of_one_root_does_not_verify_against_another() {
        let (mut trie, _) = two_leaf_trie();
        let proof = trie.prove_walk(&[0xaa, 0xbb]).unwrap();

        let mut other = BinaryTrie::new_temp();
        other.insert(vec![0xaa, 0xbb], [9u8; 32]).unwrap();
        other.insert(vec![0xaa, 0xcc], [2u8; 32]).unwrap();
        let other_root = other.commit().unwrap().root;

        assert_eq!(
            verify_walk(other_root, &[0xaa, 0xbb], &proof),
            Err(ProofError::HashMismatch)
        );
    }

    #[test]
    fn a_walk_over_a_reopened_trie_matches_the_in_memory_one() {
        // The prover re-encodes loaded nodes rather than handing back stored
        // bytes, so the two paths have to agree byte for byte or the proof
        // would depend on whether the server had the node in memory.
        let nodes = crate::trie::InMemoryBinaryTrieDB::new_empty();
        let mut trie = BinaryTrie::new(Box::new(nodes.clone()));
        for i in 0u8..16 {
            trie.insert(vec![i, i.wrapping_mul(31)], [i; 32]).unwrap();
        }
        let root = trie.commit().unwrap().root;
        let fresh = trie.prove_walk(&[7, 7u8.wrapping_mul(31)]).unwrap();

        let mut reopened = BinaryTrie::open(Box::new(nodes), root);
        let loaded = reopened.prove_walk(&[7, 7u8.wrapping_mul(31)]).unwrap();
        assert_eq!(fresh, loaded);
        assert!(verify_walk(root, &[7, 7u8.wrapping_mul(31)], &loaded).is_ok());
    }
}
