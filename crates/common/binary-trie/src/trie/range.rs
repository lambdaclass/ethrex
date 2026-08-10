//! Range proofs: prove, and check against a root alone, that a consecutive
//! run of leaves is *exactly* the tree's content over a key interval.
//!
//! The binary-trie analogue of the MPT's `verify_range`, and the primitive a
//! syncing client checks a hostile range server with. Two boundary walks
//! ([`proof`]) bracket the returned leaves; between them they name every
//! subtree that legitimately lies outside the interval, and those plus the
//! leaves are re-merkleized into a root. Equality with the root the client
//! already trusts is the whole verdict: a dropped leaf, an invented one, an
//! altered value or a moved boundary all change some branch hash on the way
//! up.
//!
//! **What the two walks each contribute.** The left walk (of the requested
//! origin) surrenders, at every split where the origin's path went *right*,
//! the subtree it stepped over — wholly below the origin, already synced or
//! never asked for. The right walk (of the last returned leaf) surrenders, at
//! every split where that key's path went *left*, the subtree it stepped over
//! — wholly above the last leaf, the not-yet-synced remainder. Neither walk
//! can hide a subtree *between* the two boundaries, because there is no step
//! that would emit one: that region is the leaves, and only the leaves.
//!
//! **What is not checked here.** That the leaves mean anything — zones,
//! sub-indices, the shape of a value — is the embedding's business and a
//! syncing client's own rule. This module knows keys, values and hashes.
//!
//! [`proof`]: super::proof

use ethereum_types::H256;
use thiserror::Error;

use crate::error::BinaryTrieError;

use super::binary_trie::BinaryTrie;
use super::bits::bytes_to_bits;
use super::node::{EMPTY_TRIE_ROOT, branch_hash, leaf_hash};
use super::proof::{ProofError, WalkEnd, WalkStep, verify_walk};

/// A served range: the leaves, and the two boundary walks that pin them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeSlice {
    /// Consecutive leaves in ascending key order, starting at the first key
    /// at or after the requested origin.
    pub leaves: Vec<(Vec<u8>, [u8; 32])>,
    /// Walk of the requested origin.
    pub left_proof: Vec<Vec<u8>>,
    /// Walk of the last returned leaf; empty exactly when `leaves` is.
    pub right_proof: Vec<Vec<u8>>,
}

/// What a verified range establishes beyond "these leaves are genuine".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedRange {
    /// The tree holds leaves beyond the last returned key, so a client
    /// syncing the whole keyspace has more to ask for.
    ///
    /// Derived from the right walk rather than taken from the server: a
    /// subtree hanging off the right of that walk *is* the remainder, and its
    /// absence means the last returned leaf is the tree's greatest.
    pub has_more: bool,
}

/// Why a range did not verify.
#[derive(Debug, Error, PartialEq, Eq, Clone, Copy)]
pub enum RangeProofError {
    /// A boundary walk is not a walk through the tree named by the root.
    #[error(transparent)]
    Proof(#[from] ProofError),
    /// The leaves are not in ascending key order, or repeat a key.
    #[error("leaf keys are not strictly ascending")]
    UnsortedLeaves,
    /// A leaf precedes the origin that was asked for.
    #[error("the first leaf precedes the requested origin")]
    LeafBeforeOrigin,
    /// The left walk ended at a leaf at or after the origin — the first leaf
    /// the tree holds in the requested range — and it is not the first leaf
    /// returned.
    #[error("the left walk's terminal is not the first returned leaf")]
    OriginMismatch,
    /// The right walk does not end at the last returned leaf, so it brackets
    /// some other key and says nothing about this range's right edge.
    #[error("the right walk does not end at the last returned leaf")]
    RightProofMismatch,
    /// The proofs show leaves at or after the origin that the response left
    /// out. An empty response is legal only when the tree holds nothing at or
    /// after the origin, and the left walk is what shows that.
    #[error("the proofs show in-range leaves the response omitted")]
    MissingLeaves,
    /// A right walk accompanied an empty range. There is no last leaf for it
    /// to be a walk of.
    #[error("unexpected right walk on an empty range")]
    UnexpectedRightProof,
    /// The leaves and the proofs re-merkleize to a different tree.
    #[error("the recomputed root does not match")]
    RootMismatch,
    /// The extracted items do not describe a tree: overlapping subtrees, or a
    /// group that cannot be split. Unreachable for anything a correct server
    /// produced, and reachable only from a hash-bound structure that already
    /// disagrees with the root.
    #[error("the range's item structure is inconsistent")]
    Malformed,
    /// The empty tree has no leaves and no nodes, so it admits only an empty
    /// range with empty proofs.
    #[error("the empty root admits only an empty range with empty proofs")]
    EmptyRootConflict,
}

/// Serve the leaves from `origin` up to `limit`, with their boundary walks.
///
/// `limit` is inclusive and *soft*: the run stops after the first leaf past
/// it, so the response always carries the terminator that proves where the
/// interval ended. `max_leaves` is a hard cap, floored at one — that floor is
/// the progress rule, which says the first leaf at or after `origin` is
/// returned if the tree holds one anywhere, whatever the budget. Without it a
/// client could not tell "the budget ran out" from "there is nothing left",
/// and an empty response would stop being provable.
///
/// The leaves come from the trie's own ordered walk, so the leaves and the
/// proofs are read from one structure. A server that sources leaves elsewhere
/// (a flat mirror) is free to, and [`verify_range`] will catch the two
/// disagreeing — the right walk has to end at the last leaf it was handed.
///
/// # Errors
///
/// [`BinaryTrieError::Backend`] or [`BinaryTrieError::MalformedNode`] if a
/// node the walk reaches could not be loaded.
pub fn prove_range(
    trie: &mut BinaryTrie,
    origin: &[u8],
    limit: &[u8],
    max_leaves: usize,
) -> Result<RangeSlice, BinaryTrieError> {
    let mut leaves = trie.leaves_from(origin, max_leaves.max(1))?;
    // The terminator: the first leaf past `limit` is included and ends the
    // run. Keeping it costs one leaf and saves the client from having to
    // trust a claim that the interval was exhausted.
    if let Some(past_limit) = leaves.iter().position(|(key, _)| key.as_slice() > limit) {
        leaves.truncate(past_limit + 1);
    }

    let left_proof = trie.prove_walk(origin)?;
    let right_proof = match leaves.last() {
        Some((key, _)) => trie.prove_walk(key)?,
        None => Vec::new(),
    };
    Ok(RangeSlice {
        leaves,
        left_proof,
        right_proof,
    })
}

/// Check that `leaves` is exactly what the tree named by `root` holds from
/// `origin` through the last returned key.
///
/// Reads nothing and needs no trie: this is what a client runs against an
/// answer from a peer it does not trust.
///
/// # Errors
///
/// Every [`RangeProofError`]; see that type for what each one means.
pub fn verify_range(
    root: H256,
    origin: &[u8],
    leaves: &[(Vec<u8>, [u8; 32])],
    left_proof: &[Vec<u8>],
    right_proof: &[Vec<u8>],
) -> Result<VerifiedRange, RangeProofError> {
    // Cheap, structural, and true of the response alone. Done first so that
    // a response that is not even a range never reaches the hashing.
    if leaves.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        return Err(RangeProofError::UnsortedLeaves);
    }
    if leaves
        .first()
        .is_some_and(|(key, _)| key.as_slice() < origin)
    {
        return Err(RangeProofError::LeafBeforeOrigin);
    }
    if root == EMPTY_TRIE_ROOT {
        if leaves.is_empty() && left_proof.is_empty() && right_proof.is_empty() {
            return Ok(VerifiedRange { has_more: false });
        }
        return Err(RangeProofError::EmptyRootConflict);
    }

    let origin_bits = bytes_to_bits(origin);
    let (left_steps, left_end) = verify_walk(root, origin, left_proof)?;

    // Everything the left walk proves lies before the origin.
    let mut items = left_side_items(&origin_bits, &left_steps);
    let terminal_precedes_origin = match &left_end {
        WalkEnd::AtLeaf { key, value } => {
            let precedes = key.as_slice() < origin;
            if precedes {
                items.push(RangeItem {
                    bits: bytes_to_bits(key),
                    hash: leaf_hash(key, value),
                });
            }
            precedes
        }
        WalkEnd::Diverged { subtree_bits, hash } => {
            let precedes = subtree_precedes_key(subtree_bits, &origin_bits);
            if precedes {
                items.push(RangeItem {
                    bits: subtree_bits.clone(),
                    hash: *hash,
                });
            }
            precedes
        }
        WalkEnd::Empty => unreachable!("only the empty root ends a walk nowhere"),
    };

    if leaves.is_empty() {
        if !right_proof.is_empty() {
            return Err(RangeProofError::UnexpectedRightProof);
        }
        // The claim is "the tree holds nothing at or after the origin". A
        // step that went *left* stepped over a subtree on the origin's right,
        // which is content at or after it; and a terminal that does not
        // precede the origin is itself such content.
        if left_steps.iter().any(|step| step.taken == 0) || !terminal_precedes_origin {
            return Err(RangeProofError::MissingLeaves);
        }
        // The left side alone must then be the whole tree.
        check_recomputed_root(root, &items)?;
        return Ok(VerifiedRange { has_more: false });
    }

    let (last_key, last_value) = leaves.last().expect("leaves are not empty");
    let (right_steps, right_end) = verify_walk(root, last_key, right_proof)?;
    if right_end
        != (WalkEnd::AtLeaf {
            key: last_key.clone(),
            value: *last_value,
        })
    {
        return Err(RangeProofError::RightProofMismatch);
    }

    // A left walk that ends at a leaf at or after the origin has found the
    // *first* such leaf: any key between the two would have branched off this
    // very walk, on the side the walk did not take. So it is the first leaf
    // the response owes.
    if let WalkEnd::AtLeaf { key, value } = &left_end
        && key.as_slice() >= origin
        && (key, value) != (&leaves[0].0, &leaves[0].1)
    {
        return Err(RangeProofError::OriginMismatch);
    }

    items.extend(leaves.iter().map(|(key, value)| RangeItem {
        bits: bytes_to_bits(key),
        hash: leaf_hash(key, value),
    }));

    let right_items = right_side_items(&bytes_to_bits(last_key), &right_steps);
    let has_more = !right_items.is_empty();
    items.extend(right_items);

    check_recomputed_root(root, &items)?;
    Ok(VerifiedRange { has_more })
}

/// A completed piece of the tree, at a known position: either a returned leaf
/// or a subtree one of the walks named without opening.
///
/// One type for both because the recomputation treats them identically — the
/// hash is used as it stands, and the bits say where. A leaf's encoding
/// carries its whole key and a branch's hash commits to everything below it,
/// so in neither case does anything below the item's position still need
/// saying.
struct RangeItem {
    /// Position: the bit string this item covers, from the root.
    bits: Vec<u8>,
    /// Commitment to the item's whole content.
    hash: H256,
}

/// Subtrees the left walk stepped over, ascending.
///
/// At a split the origin's path took the 1 side of, the 0-side subtree holds
/// only keys that agree with the origin down to the split and are 0 where it
/// is 1 — every one of them below the origin.
///
/// Already in ascending order: a shallower step's sibling disagrees with the
/// origin earlier, and disagrees downwards.
fn left_side_items(origin_bits: &[u8], steps: &[WalkStep]) -> Vec<RangeItem> {
    steps
        .iter()
        .filter(|step| step.taken == 1)
        .map(|step| RangeItem {
            bits: extend(&origin_bits[..step.split], 0),
            hash: step.sibling,
        })
        .collect()
}

/// Subtrees the right walk stepped over, ascending — the remainder of the
/// keyspace above the last returned leaf.
///
/// Mirror of [`left_side_items`], with the sides swapped and one difference
/// that is not symmetry: the walk produces these in *descending* order. A
/// shallower step's sibling disagrees with the last key earlier and disagrees
/// *upwards*, which puts it further right, so the deepest step's sibling — the
/// nearest subtree after the last leaf — comes first in key order.
fn right_side_items(last_bits: &[u8], steps: &[WalkStep]) -> Vec<RangeItem> {
    steps
        .iter()
        .rev()
        .filter(|step| step.taken == 0)
        .map(|step| RangeItem {
            bits: extend(&last_bits[..step.split], 1),
            hash: step.sibling,
        })
        .collect()
}

fn extend(bits: &[u8], bit: u8) -> Vec<u8> {
    let mut extended = Vec::with_capacity(bits.len() + 1);
    extended.extend_from_slice(bits);
    extended.push(bit);
    extended
}

/// Whether every key under `subtree_bits` sorts before `key_bits`.
///
/// The first bit they disagree on decides, as it does for any two bit
/// strings. Agreement over the whole overlap means the subtree covers the
/// key's entire bit prefix, so every key beneath it *extends* the target and
/// therefore sorts after it — the case a walk that ran out of key bits inside
/// a branch prefix lands in. The converse shape, a subtree whose bits are a
/// proper prefix of the key's and agree, cannot reach here: that is a subtree
/// the walk would have descended into rather than stopped at.
fn subtree_precedes_key(subtree_bits: &[u8], key_bits: &[u8]) -> bool {
    subtree_bits
        .iter()
        .zip(key_bits)
        .find(|(subtree, key)| subtree != key)
        .is_some_and(|(subtree, key)| subtree < key)
}

/// Rebuild the root over `items` and compare it with the one the client
/// already trusts.
///
/// Ordering is checked *here* rather than left to the fold. It is tempting to
/// leave it: the fold rejects most disordered lists on its own, since a group
/// whose ends bracket nothing has no bit to split at. It does not reject all
/// of them — `check_recomputed_root_rejects_every_non_frontier` exhibits one
/// it folds happily — and the difference matters, because on that path the
/// answer would come back `RootMismatch`, blaming a peer for the shape of a
/// list this side built.
///
/// Ordering only, and not the rest of the frontier property: a list in which
/// one item's bits are a *prefix* of the next's is ascending and still not a
/// frontier, and the fold already answers `Malformed` for it — there is no bit
/// inside the overlap for those two to split at. Restating that here would be
/// a condition no input could distinguish.
fn check_recomputed_root(root: H256, items: &[RangeItem]) -> Result<(), RangeProofError> {
    if items.windows(2).any(|pair| pair[0].bits >= pair[1].bits) {
        return Err(RangeProofError::Malformed);
    }
    if recompute(items, 0)? != root {
        return Err(RangeProofError::RootMismatch);
    }
    Ok(())
}

/// Rebuild the root over a sorted frontier of items.
///
/// `BinaryTrie::from_sorted_leaves`'s fold, generalized from leaves to items:
/// a group of one is its own hash, and a larger group is a branch splitting at
/// the first bit its ends disagree on. That the two constructions are the same
/// recursion is what makes "the recomputed root equals the real one" mean the
/// tree really has this shape.
///
/// `depth` is the bit index the group's parent split at, so the branch prefix
/// built here runs from `depth` to the group's own split.
fn recompute(items: &[RangeItem], depth: usize) -> Result<H256, RangeProofError> {
    if let [only] = items {
        return Ok(only.hash);
    }
    let (Some(first), Some(last)) = (items.first(), items.last()) else {
        unreachable!("groups are non-empty: the split boundary is checked to be interior")
    };
    // Sorted, so the ends bracket the group: whatever they share, all of it
    // shares. An item whose bits run out before the group splits would be a
    // position containing another item's, which is not a frontier.
    let overlap = first.bits.len().min(last.bits.len());
    let split = (depth..overlap)
        .find(|&bit| first.bits[bit] != last.bits[bit])
        .ok_or(RangeProofError::Malformed)?;
    if items.iter().any(|item| item.bits.len() <= split) {
        return Err(RangeProofError::Malformed);
    }
    let boundary = items.partition_point(|item| item.bits[split] == 0);
    if boundary == 0 || boundary == items.len() {
        return Err(RangeProofError::Malformed);
    }
    Ok(branch_hash(
        &first.bits[depth..split],
        recompute(&items[..boundary], split + 1)?,
        recompute(&items[boundary..], split + 1)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trie::increment_key;

    /// Four two-byte keys, spread so the tree branches at several depths.
    const KEYS: [[u8; 2]; 4] = [[0x00, 0x10], [0x00, 0x20], [0x80, 0x01], [0xf0, 0x0f]];

    fn value(i: usize) -> [u8; 32] {
        [i as u8 + 1; 32]
    }

    fn built(keys: &[[u8; 2]]) -> (BinaryTrie, H256) {
        let mut trie = BinaryTrie::new_temp();
        for (i, key) in keys.iter().enumerate() {
            trie.insert(key.to_vec(), value(i)).unwrap();
        }
        let root = trie.commit().unwrap().root;
        (trie, root)
    }

    fn sample() -> (BinaryTrie, H256) {
        built(&KEYS)
    }

    fn all_leaves() -> Vec<(Vec<u8>, [u8; 32])> {
        KEYS.iter()
            .enumerate()
            .map(|(i, key)| (key.to_vec(), value(i)))
            .collect()
    }

    const MAX: [u8; 2] = [0xff, 0xff];
    const MIN: [u8; 2] = [0x00, 0x00];

    fn check(root: H256, origin: &[u8], slice: &RangeSlice) -> Result<bool, RangeProofError> {
        verify_range(
            root,
            origin,
            &slice.leaves,
            &slice.left_proof,
            &slice.right_proof,
        )
        .map(|verified| verified.has_more)
    }

    #[test]
    fn an_empty_tree_admits_only_an_empty_range() {
        let mut trie = BinaryTrie::new_temp();
        let slice = prove_range(&mut trie, &MIN, &MAX, 10).unwrap();
        assert!(slice.leaves.is_empty());
        assert!(slice.left_proof.is_empty() && slice.right_proof.is_empty());
        assert_eq!(check(EMPTY_TRIE_ROOT, &MIN, &slice), Ok(false));

        // Anything else against the empty root is a fabrication.
        assert_eq!(
            verify_range(EMPTY_TRIE_ROOT, &MIN, &[(vec![0x01], [1u8; 32])], &[], &[]),
            Err(RangeProofError::EmptyRootConflict)
        );
        assert_eq!(
            verify_range(EMPTY_TRIE_ROOT, &MIN, &[], &[vec![0x00]], &[]),
            Err(RangeProofError::EmptyRootConflict)
        );
        assert_eq!(
            verify_range(EMPTY_TRIE_ROOT, &MIN, &[], &[], &[vec![0x00]]),
            Err(RangeProofError::EmptyRootConflict)
        );
    }

    #[test]
    fn a_whole_range_round_trips_and_reports_no_more() {
        let (mut trie, root) = sample();
        let slice = prove_range(&mut trie, &MIN, &MAX, 10).unwrap();
        assert_eq!(slice.leaves, all_leaves());
        assert_eq!(check(root, &MIN, &slice), Ok(false));
    }

    #[test]
    fn a_budgeted_range_reports_more_and_resumes_to_completion() {
        let (mut trie, root) = sample();
        let mut collected = Vec::new();
        let mut origin = MIN.to_vec();
        loop {
            let slice = prove_range(&mut trie, &origin, &MAX, 2).unwrap();
            assert!(slice.leaves.len() <= 2, "the budget is a hard cap");
            let has_more = check(root, &origin, &slice).unwrap();
            let last = slice.leaves.last().expect("progress").0.clone();
            collected.extend(slice.leaves);
            if !has_more {
                break;
            }
            origin = increment_key(&last).expect("not the maximal key");
        }
        assert_eq!(collected, all_leaves(), "the chained slices are the tree");
    }

    #[test]
    fn a_middle_slice_verifies_against_an_origin_between_keys() {
        let (mut trie, root) = sample();
        // Between KEYS[0] and KEYS[1]: the left walk ends at a leaf that is
        // *after* the origin, which is the exclusion shape.
        let origin = [0x00, 0x18];
        let slice = prove_range(&mut trie, &origin, &MAX, 10).unwrap();
        assert_eq!(slice.leaves.len(), 3);
        assert_eq!(slice.leaves[0].0, KEYS[1].to_vec());
        assert_eq!(check(root, &origin, &slice), Ok(false));
    }

    #[test]
    fn a_limit_between_keys_still_returns_the_terminator() {
        let (mut trie, root) = sample();
        // The limit falls between the second and third keys, so the third is
        // returned anyway: the terminator is what shows the interval ended.
        let limit = [0x40, 0x00];
        let slice = prove_range(&mut trie, &MIN, &limit, 10).unwrap();
        assert_eq!(slice.leaves.len(), 3);
        assert_eq!(slice.leaves[2].0, KEYS[2].to_vec());
        assert_eq!(check(root, &MIN, &slice), Ok(true), "one key is left");
    }

    #[test]
    fn a_zero_budget_still_returns_the_first_leaf() {
        // The progress rule: an empty response has to mean "nothing here".
        let (mut trie, root) = sample();
        let slice = prove_range(&mut trie, &MIN, &MAX, 0).unwrap();
        assert_eq!(slice.leaves.len(), 1);
        assert_eq!(check(root, &MIN, &slice), Ok(true));
    }

    #[test]
    fn an_origin_past_every_key_proves_emptiness() {
        let (mut trie, root) = sample();
        let origin = [0xff, 0xf0];
        let slice = prove_range(&mut trie, &origin, &MAX, 10).unwrap();
        assert!(slice.leaves.is_empty());
        assert!(slice.right_proof.is_empty());
        assert!(!slice.left_proof.is_empty(), "emptiness is still proven");
        assert_eq!(check(root, &origin, &slice), Ok(false));
    }

    #[test]
    fn forged_emptiness_is_rejected() {
        let (mut trie, root) = sample();
        // A server answering "nothing at or after the origin" while the tree
        // plainly holds keys there: the left walk it must supply is the walk
        // that contradicts it.
        let slice = prove_range(&mut trie, &MIN, &MAX, 10).unwrap();
        assert_eq!(
            verify_range(root, &MIN, &[], &slice.left_proof, &[]),
            Err(RangeProofError::MissingLeaves)
        );

        // Also from an origin in the middle, where the emptiness claim needs
        // the terminal check rather than the step check to die.
        let origin = [0xf0, 0x00];
        let middle = prove_range(&mut trie, &origin, &MAX, 10).unwrap();
        assert_eq!(
            verify_range(root, &origin, &[], &middle.left_proof, &[]),
            Err(RangeProofError::MissingLeaves)
        );

        // And the case only the *step* check can catch: an origin whose walk
        // ends at a leaf genuinely below it — so the terminal is no evidence
        // of withholding — while a shallower step went left, stepping over a
        // whole subtree of keys above the origin. That subtree is the
        // withheld content, and the step is the only thing that names it.
        let origin = [0x00, 0x30];
        let stepped = prove_range(&mut trie, &origin, &MAX, 10).unwrap();
        let (steps, end) = verify_walk(root, &origin, &stepped.left_proof).unwrap();
        assert!(
            steps.iter().any(|step| step.taken == 0),
            "the shape this case needs"
        );
        assert!(
            matches!(&end, WalkEnd::AtLeaf { key, .. } if key.as_slice() < &origin[..]),
            "a terminal that precedes the origin, so only the steps object"
        );
        assert_eq!(
            verify_range(root, &origin, &[], &stepped.left_proof, &[]),
            Err(RangeProofError::MissingLeaves)
        );
    }

    #[test]
    fn a_subtree_wholly_below_the_origin_is_a_left_item() {
        // The left walk's terminal is a *branch* the origin diverged past on
        // the high side, so the whole subtree under it is behind the origin
        // and has to be carried into the recomputation as one item. Nothing
        // else in this module produces that shape: it needs an origin that
        // leaves a branch's covered bits upwards rather than landing in it.
        let (mut trie, root) = built(&[[0x00, 0x10], [0x00, 0x20], [0x80, 0x01]]);
        let origin = [0x00, 0x40];
        let (_, end) = verify_walk(root, &origin, &trie.prove_walk(&origin).unwrap()).unwrap();
        assert!(
            matches!(end, WalkEnd::Diverged { .. }),
            "the shape this case needs"
        );

        let slice = prove_range(&mut trie, &origin, &MAX, 10).unwrap();
        assert_eq!(slice.leaves.len(), 1, "only the 0x80 key is at or after it");
        assert_eq!(check(root, &origin, &slice), Ok(false));

        // The same tree with nothing above the origin: now the diverged
        // subtree is the entire tree, and proving emptiness rests on it alone.
        let (mut small, small_root) = built(&[[0x00, 0x10], [0x00, 0x20]]);
        let empty = prove_range(&mut small, &origin, &MAX, 10).unwrap();
        assert!(empty.leaves.is_empty());
        assert_eq!(check(small_root, &origin, &empty), Ok(false));
    }

    #[test]
    fn an_empty_range_may_not_carry_a_right_walk() {
        let (mut trie, root) = sample();
        let origin = [0xff, 0xf0];
        let slice = prove_range(&mut trie, &origin, &MAX, 10).unwrap();
        let stray = trie.prove_walk(&KEYS[0]).unwrap();
        assert_eq!(
            verify_range(root, &origin, &[], &slice.left_proof, &stray),
            Err(RangeProofError::UnexpectedRightProof)
        );
    }

    #[test]
    fn a_dropped_middle_leaf_is_a_root_mismatch() {
        let (mut trie, root) = sample();
        let slice = prove_range(&mut trie, &MIN, &MAX, 10).unwrap();
        let mut gapped = slice.leaves.clone();
        gapped.remove(1);
        assert_eq!(
            verify_range(root, &MIN, &gapped, &slice.left_proof, &slice.right_proof),
            Err(RangeProofError::RootMismatch)
        );
    }

    #[test]
    fn an_injected_leaf_is_rejected() {
        let (mut trie, root) = sample();
        let slice = prove_range(&mut trie, &MIN, &MAX, 10).unwrap();
        let mut padded = slice.leaves.clone();
        padded.insert(2, (vec![0x40, 0x40], [9u8; 32]));
        assert_eq!(
            verify_range(root, &MIN, &padded, &slice.left_proof, &slice.right_proof),
            Err(RangeProofError::RootMismatch)
        );
    }

    #[test]
    fn a_tampered_value_is_a_root_mismatch() {
        let (mut trie, root) = sample();
        let slice = prove_range(&mut trie, &MIN, &MAX, 10).unwrap();
        for index in 0..slice.leaves.len() {
            let mut altered = slice.leaves.clone();
            altered[index].1[0] ^= 1;
            // The two boundary leaves are named by their own walks, key and
            // value both, so they are caught by the more specific boundary
            // check before any hashing. Only the interior is left to the root
            // recomputation — which is the half that has to be airtight,
            // since it is the only thing standing behind those leaves.
            let expected = match index {
                0 => RangeProofError::OriginMismatch,
                i if i + 1 == slice.leaves.len() => RangeProofError::RightProofMismatch,
                _ => RangeProofError::RootMismatch,
            };
            assert_eq!(
                verify_range(root, &MIN, &altered, &slice.left_proof, &slice.right_proof),
                Err(expected),
                "altering leaf {index}"
            );
        }
    }

    #[test]
    fn a_tampered_key_is_a_root_mismatch() {
        let (mut trie, root) = sample();
        // Move a middle leaf's key without moving it out of order, so the
        // sortedness precondition cannot be what catches it.
        let slice = prove_range(&mut trie, &MIN, &MAX, 10).unwrap();
        let mut altered = slice.leaves.clone();
        altered[1].0 = vec![0x00, 0x21];
        assert_eq!(
            verify_range(root, &MIN, &altered, &slice.left_proof, &slice.right_proof),
            Err(RangeProofError::RootMismatch)
        );
    }

    #[test]
    fn unsorted_or_repeated_leaves_are_rejected() {
        let (mut trie, root) = sample();
        let slice = prove_range(&mut trie, &MIN, &MAX, 10).unwrap();

        let mut swapped = slice.leaves.clone();
        swapped.swap(1, 2);
        assert_eq!(
            verify_range(root, &MIN, &swapped, &slice.left_proof, &slice.right_proof),
            Err(RangeProofError::UnsortedLeaves)
        );

        let mut repeated = slice.leaves.clone();
        repeated.insert(1, slice.leaves[1].clone());
        assert_eq!(
            verify_range(root, &MIN, &repeated, &slice.left_proof, &slice.right_proof),
            Err(RangeProofError::UnsortedLeaves)
        );
    }

    #[test]
    fn a_leaf_before_the_origin_is_rejected() {
        let (mut trie, root) = sample();
        let origin = KEYS[1];
        let slice = prove_range(&mut trie, &origin, &MAX, 10).unwrap();
        let mut reaching_back = slice.leaves.clone();
        reaching_back.insert(0, (KEYS[0].to_vec(), value(0)));
        assert_eq!(
            verify_range(
                root,
                &origin,
                &reaching_back,
                &slice.left_proof,
                &slice.right_proof
            ),
            Err(RangeProofError::LeafBeforeOrigin)
        );
    }

    #[test]
    fn the_first_leaf_must_be_the_one_the_left_walk_found() {
        let (mut trie, root) = sample();
        // Origin exactly on a key, so the left walk terminates at that leaf
        // and knows precisely which leaf the response owes first.
        let origin = KEYS[0];
        let slice = prove_range(&mut trie, &origin, &MAX, 10).unwrap();
        let skipped = slice.leaves[1..].to_vec();
        assert_eq!(
            verify_range(
                root,
                &origin,
                &skipped,
                &slice.left_proof,
                &slice.right_proof
            ),
            Err(RangeProofError::OriginMismatch)
        );

        // Same leaf, wrong value: the walk pins the value too.
        let mut restated = slice.leaves.clone();
        restated[0].1[0] ^= 1;
        assert_eq!(
            verify_range(
                root,
                &origin,
                &restated,
                &slice.left_proof,
                &slice.right_proof
            ),
            Err(RangeProofError::OriginMismatch)
        );
    }

    #[test]
    fn the_right_walk_must_be_a_walk_of_the_last_leaf() {
        let (mut trie, root) = sample();
        let slice = prove_range(&mut trie, &MIN, &MAX, 10).unwrap();

        // A leaf the tree does not hold, with a genuine walk of its key. The
        // walk verifies — it is a perfectly good exclusion proof — and it is
        // the terminal check that catches the invention. This is the only way
        // to reach RightProofMismatch: a right walk of a key the tree *does*
        // hold cannot end anywhere but at that key.
        let invented = vec![0xff, 0x00];
        let mut padded = slice.leaves.clone();
        padded.push((invented.clone(), [7u8; 32]));
        let genuine_walk = trie.prove_walk(&invented).unwrap();
        assert_eq!(
            verify_range(root, &MIN, &padded, &slice.left_proof, &genuine_walk),
            Err(RangeProofError::RightProofMismatch)
        );

        // A walk of a different key the tree really holds: genuine nodes,
        // wrong path, so the hash chain is what breaks.
        let elsewhere = trie.prove_walk(&KEYS[1]).unwrap();
        assert_eq!(
            verify_range(root, &MIN, &slice.leaves, &slice.left_proof, &elsewhere),
            Err(RangeProofError::Proof(ProofError::HashMismatch))
        );
    }

    #[test]
    fn swapped_boundary_walks_are_rejected() {
        let (mut trie, root) = sample();
        let origin = [0x00, 0x18];
        let slice = prove_range(&mut trie, &origin, &MAX, 10).unwrap();
        assert!(
            verify_range(
                root,
                &origin,
                &slice.leaves,
                &slice.right_proof,
                &slice.left_proof
            )
            .is_err()
        );
    }

    #[test]
    fn a_tampered_boundary_node_is_rejected() {
        let (mut trie, root) = sample();
        let origin = [0x00, 0x18];
        let slice = prove_range(&mut trie, &origin, &MAX, 10).unwrap();
        for side in 0..2 {
            let proof = if side == 0 {
                &slice.left_proof
            } else {
                &slice.right_proof
            };
            for node in 0..proof.len() {
                for byte in 0..proof[node].len() {
                    let mut tampered = proof.clone();
                    tampered[node][byte] ^= 1;
                    let (left, right) = if side == 0 {
                        (&tampered, &slice.right_proof)
                    } else {
                        (&slice.left_proof, &tampered)
                    };
                    assert!(
                        verify_range(root, &origin, &slice.leaves, left, right).is_err(),
                        "side {side} node {node} byte {byte} was accepted"
                    );
                }
            }
        }
    }

    #[test]
    fn a_truncated_boundary_walk_is_rejected() {
        let (mut trie, root) = sample();
        let slice = prove_range(&mut trie, &MIN, &MAX, 10).unwrap();
        let short_left = &slice.left_proof[..slice.left_proof.len() - 1];
        assert_eq!(
            verify_range(root, &MIN, &slice.leaves, short_left, &slice.right_proof),
            Err(RangeProofError::Proof(ProofError::Truncated))
        );
        let short_right = &slice.right_proof[..slice.right_proof.len() - 1];
        assert_eq!(
            verify_range(root, &MIN, &slice.leaves, &slice.left_proof, short_right),
            Err(RangeProofError::Proof(ProofError::Truncated))
        );
    }

    #[test]
    fn a_slice_does_not_verify_against_another_tree() {
        let (mut trie, _) = sample();
        let slice = prove_range(&mut trie, &MIN, &MAX, 10).unwrap();
        // Same keys, one different value: a fully self-consistent state,
        // served for the wrong root.
        let (_, other_root) = built(&[KEYS[0], KEYS[1], KEYS[2]]);
        assert!(
            verify_range(
                other_root,
                &MIN,
                &slice.leaves,
                &slice.left_proof,
                &slice.right_proof
            )
            .is_err()
        );
    }

    #[test]
    fn a_single_leaf_tree_verifies_from_its_own_key() {
        let (mut trie, root) = built(&[KEYS[2]]);
        for origin in [MIN, KEYS[2], MAX] {
            let slice = prove_range(&mut trie, &origin, &MAX, 10).unwrap();
            let expected_leaves = usize::from(origin <= KEYS[2]);
            assert_eq!(slice.leaves.len(), expected_leaves, "origin {origin:?}");
            assert_eq!(check(root, &origin, &slice), Ok(false));
        }
    }

    #[test]
    fn an_empty_origin_means_from_the_beginning() {
        // The left walk of an empty key diverges at the root, which is the
        // "nothing is before the origin" shape.
        let (mut trie, root) = sample();
        let slice = prove_range(&mut trie, &[], &MAX, 10).unwrap();
        assert_eq!(slice.leaves, all_leaves());
        assert_eq!(check(root, &[], &slice), Ok(false));
    }

    #[test]
    fn a_two_leaf_tree_split_at_the_first_bit() {
        // The left walk's terminal *is* the root branch, so the whole range
        // rests on a single Diverged item and no steps at all.
        let (mut trie, root) = built(&[[0x00, 0x01], [0x80, 0x01]]);
        let origin = [0x40, 0x00];
        let slice = prove_range(&mut trie, &origin, &MAX, 10).unwrap();
        assert_eq!(slice.leaves.len(), 1, "only the 0x80 key follows");
        assert_eq!(check(root, &origin, &slice), Ok(false));

        // And from below everything, where nothing precedes the origin.
        let whole = prove_range(&mut trie, &MIN, &MAX, 10).unwrap();
        assert_eq!(whole.leaves.len(), 2);
        assert_eq!(check(root, &MIN, &whole), Ok(false));
    }

    /// The recomputation's structural guards, exercised directly.
    ///
    /// Nothing [`verify_range`] accepts can reach them: an item list is built
    /// from hash-bound walk data plus leaves that have already been checked
    /// sorted, at or after the origin, and bracketed by the two walks, and
    /// every overlapping shape is excluded by one of those. They exist so
    /// that being wrong about that argument costs an error rather than an
    /// index-out-of-bounds panic on data a peer supplied — which is the one
    /// failure mode a range verifier must not have. Tested here rather than
    /// through the public entry point precisely because there is no input to
    /// the public entry point that would.
    mod recomputation_guards {
        use super::*;

        fn item(bits: &[u8], byte: u8) -> RangeItem {
            RangeItem {
                bits: bits.to_vec(),
                hash: H256([byte; 32]),
            }
        }

        #[test]
        fn an_item_containing_another_is_malformed() {
            // [0, 1] covers everything [0, 1, 0] covers, so the two are not a
            // frontier and the group cannot be split at a bit both have.
            assert_eq!(
                recompute(&[item(&[0, 1], 1), item(&[0, 1, 0], 2)], 0),
                Err(RangeProofError::Malformed)
            );
        }

        #[test]
        fn a_lone_item_is_its_own_hash() {
            assert_eq!(recompute(&[item(&[0, 1], 5)], 0), Ok(H256([5u8; 32])));
        }

        /// Every list of up to three short bit strings, in every order.
        ///
        /// Two properties, exhaustively:
        ///
        /// 1. **Total** — an answer, never a panic, on any list at all. That
        ///    is what the fold's guards buy, and the one behaviour a range
        ///    verifier must not lack on data a peer supplied.
        /// 2. **Every non-frontier is `Malformed`** — never `RootMismatch`.
        ///    The fold alone does not give this: `[[0,0], [1,1,0], [0,1,1]]`
        ///    splits at bit 1, lands one item left and two right, and folds to
        ///    a perfectly good hash of a tree that is not the one those
        ///    positions describe. Under a random root that would come back
        ///    `RootMismatch` — blaming a peer for a list this side assembled.
        ///
        /// Neither is testable through [`verify_range`], whose item lists are
        /// always frontiers: the leaves are checked ascending, the left items
        /// all lie below the origin and the right items all above the last
        /// leaf. These paths open only where `partition_point`'s answer is
        /// unspecified — which is exactly where a hostile peer would aim.
        ///
        /// Exhaustive rather than random: the domain is 2 940 lists, smaller
        /// than a useful proptest budget and not dependent on a seed to cover
        /// the shapes that matter.
        #[test]
        fn check_recomputed_root_rejects_every_non_frontier() {
            let (mut frontiers, mut malformed) = (0usize, 0usize);
            // No item list of distinct hashes folds to this, so a frontier
            // reaches the comparison and fails there.
            let unreachable_root = H256::repeat_byte(0xa5);
            for list in small_item_lists() {
                let items = as_items(&list);
                match check_recomputed_root(unreachable_root, &items) {
                    Err(RangeProofError::RootMismatch) => {
                        assert!(is_frontier(&list), "a non-frontier was folded: {list:?}");
                        frontiers += 1;
                    }
                    Err(RangeProofError::Malformed) => malformed += 1,
                    other => panic!("unexpected outcome {other:?} for {list:?}"),
                }
            }
            // Both outcomes are exercised, so neither half is vacuous.
            assert!(frontiers > 0 && malformed > 0);
        }

        /// The fold on its own, over the same domain, with nothing filtering
        /// its input first.
        ///
        /// This is where the fold's two internal guards are exercised. They
        /// are unreachable through [`check_recomputed_root`] — the ordering
        /// check in front of it turns the lists that would reach them away —
        /// and unreachable through [`verify_range`] twice over. What they are
        /// for is that being wrong about either of those arguments should
        /// cost an error rather than an index-out-of-bounds panic on bytes a
        /// peer chose. So the property is exactly *totality*, and it has to be
        /// asserted against the fold directly or not at all.
        #[test]
        fn the_fold_is_total_over_every_small_item_list() {
            let mut folded = 0usize;
            for list in small_item_lists() {
                match recompute(&as_items(&list), 0) {
                    Ok(_) | Err(RangeProofError::Malformed) => folded += 1,
                    other => panic!("unexpected outcome {other:?} for {list:?}"),
                }
            }
            assert_eq!(folded, 14 * 14 + 14 * 14 * 14);
        }

        /// Every list of two or three bit strings of one to three bits.
        ///
        /// Exhaustive rather than random: 2 940 lists is smaller than a useful
        /// proptest budget and does not depend on a seed to reach the shapes
        /// that matter.
        fn small_item_lists() -> Vec<Vec<Vec<u8>>> {
            let vocabulary: Vec<Vec<u8>> = (1..=3)
                .flat_map(|len| {
                    (0..1u32 << len)
                        .map(move |n| (0..len).map(|b| ((n >> (len - 1 - b)) & 1) as u8).collect())
                })
                .collect();
            assert_eq!(vocabulary.len(), 14);

            let mut lists = Vec::new();
            for first in &vocabulary {
                for second in &vocabulary {
                    lists.push(vec![first.clone(), second.clone()]);
                    for third in &vocabulary {
                        lists.push(vec![first.clone(), second.clone(), third.clone()]);
                    }
                }
            }
            assert_eq!(lists.len(), 14 * 14 + 14 * 14 * 14);
            lists
        }

        fn as_items(list: &[Vec<u8>]) -> Vec<RangeItem> {
            list.iter()
                .enumerate()
                .map(|(i, bits)| item(bits, i as u8))
                .collect()
        }

        /// Strictly ascending and prefix-free: positions that name disjoint
        /// pieces of one tree, in order.
        fn is_frontier(list: &[Vec<u8>]) -> bool {
            list.windows(2)
                .all(|pair| pair[0] < pair[1] && !pair[1].starts_with(&pair[0]))
        }
    }

    #[test]
    fn every_sub_range_of_the_sample_verifies() {
        let (mut trie, root) = sample();
        let leaves = all_leaves();
        for first in 0..leaves.len() {
            for last in first..leaves.len() {
                let origin = leaves[first].0.clone();
                let slice =
                    prove_range(&mut trie, &origin, &leaves[last].0, last - first + 1).unwrap();
                assert_eq!(slice.leaves, leaves[first..=last], "[{first}, {last}]");
                assert_eq!(
                    check(root, &origin, &slice),
                    Ok(last + 1 < leaves.len()),
                    "[{first}, {last}]"
                );
            }
        }
    }
}
