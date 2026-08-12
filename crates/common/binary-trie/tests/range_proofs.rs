//! Range proofs against ground truth, and against a hostile server.
//!
//! **No new spec vectors.** Range proofs are this crate's construction — no
//! EELS reference exists for them — so there is nothing to conform to. What
//! there is instead are two oracles the crate already trusts:
//!
//! 1. the tries in `vectors/binary_trie_vectors.json`, whose roots
//!    `spec_vectors.rs` pins against the EELS reference. Every range here is
//!    verified against the **pinned** root read from the fixture, never
//!    against a root this code computed — so a bug that moved both the tree
//!    and the proof the same way would still be caught;
//! 2. `BinaryTrie::from_sorted_leaves` as a rebuild oracle under random
//!    workloads, since a range proof is a partial re-run of exactly that fold.
//!
//! And then the part that matters most: a range proof exists to be checked
//! against a server with every reason to lie. `adversarial_*` grinds the
//! surface — tampered proof bytes, dropped and injected leaves, swapped
//! boundaries, forged emptiness, a self-consistent state served for the wrong
//! root — and asserts rejection, not merely that honest input is accepted.

use std::collections::BTreeMap;

use ethereum_types::H256;
use ethrex_binary_trie::trie::{
    BinaryTrie, EMPTY_TRIE_ROOT, InMemoryBinaryTrieDB, ProofError, RangeProofError, RangeSlice,
    increment_key, prove_range, verify_range,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    trie_roots: Vec<TrieCase>,
}

#[derive(Deserialize)]
struct TrieCase {
    name: String,
    entries: Vec<Entry>,
    root: String,
}

#[derive(Deserialize)]
struct Entry {
    key: String,
    value: String,
}

fn unhex(s: &str) -> Vec<u8> {
    hex::decode(s.strip_prefix("0x").unwrap_or(s)).expect("fixture hex string")
}

/// A vector trie: its case name, its distinct leaves in key order, and the
/// root the fixture pins against the EELS reference.
type VectorTrie = (String, Vec<(Vec<u8>, [u8; 32])>, H256);

/// The vector tries, each as (name, sorted distinct leaves, pinned root).
///
/// The last-write-wins reduction is the one `spec_vectors.rs` applies for the
/// same reason: `overwrite_takes_last_value` repeats a key deliberately, and
/// the surviving value per key is the shape a real leaf set has.
fn vector_tries() -> Vec<VectorTrie> {
    let fixture: Fixture =
        serde_json::from_str(include_str!("vectors/binary_trie_vectors.json")).unwrap();
    assert!(!fixture.trie_roots.is_empty(), "no trie root cases");
    fixture
        .trie_roots
        .into_iter()
        .map(|case| {
            let mut surviving: BTreeMap<Vec<u8>, [u8; 32]> = BTreeMap::new();
            for entry in &case.entries {
                surviving.insert(
                    unhex(&entry.key),
                    unhex(&entry.value).try_into().expect("32-byte value"),
                );
            }
            let root = H256::from_slice(&unhex(&case.root));
            (case.name, surviving.into_iter().collect(), root)
        })
        .collect()
}

fn built(leaves: &[(Vec<u8>, [u8; 32])]) -> BinaryTrie {
    let mut trie = BinaryTrie::new(Box::new(InMemoryBinaryTrieDB::new_empty()));
    for (key, value) in leaves {
        trie.insert(key.clone(), *value).unwrap();
    }
    trie.commit().unwrap();
    trie
}

/// The bytewise predecessor: the exact inverse of `increment_key`, and the
/// origin that makes a request an *exclusion* on its left boundary while still
/// naming the same first leaf.
fn decrement_key(key: &[u8]) -> Option<Vec<u8>> {
    let mut previous = key.to_vec();
    for byte in previous.iter_mut().rev() {
        let (decremented, borrowed) = byte.overflowing_sub(1);
        *byte = decremented;
        if !borrowed {
            return Some(previous);
        }
    }
    None
}

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

/// Every sub-range of every spec-vector trie, verified against the root the
/// fixture pins — including one perturbed-origin variant per range.
#[test]
fn spec_vector_tries_verify_every_sub_range() {
    for (name, leaves, root) in vector_tries() {
        let mut trie = built(&leaves);
        assert_eq!(trie.root(), root, "trie case {name} does not build");

        if leaves.is_empty() {
            assert_eq!(root, EMPTY_TRIE_ROOT, "case {name}");
            let slice = prove_range(&mut trie, &[], &[0xff; 66], 100).unwrap();
            assert_eq!(check(root, &[], &slice), Ok(false), "case {name}");
            continue;
        }

        for first in 0..leaves.len() {
            for last in first..leaves.len() {
                let budget = last - first + 1;
                let limit = &leaves[last].0;
                let expect_more = last + 1 < leaves.len();

                let origin = leaves[first].0.clone();
                let slice = prove_range(&mut trie, &origin, limit, budget).unwrap();
                assert_eq!(
                    slice.leaves,
                    leaves[first..=last],
                    "case {name} range [{first}, {last}]"
                );
                assert_eq!(
                    check(root, &origin, &slice),
                    Ok(expect_more),
                    "case {name} range [{first}, {last}]"
                );

                // The same range asked for from just below its first key: the
                // left walk is now an exclusion proof, and nothing sorts
                // between the two, so the answer must not move — unless the
                // predecessor is itself a key, which is a different range and
                // not this assertion's business. `two_leaves_diverge_last_bit`
                // is exactly that: two keys one apart.
                let Some(perturbed) = decrement_key(&origin) else {
                    continue;
                };
                if leaves.iter().any(|(key, _)| key == &perturbed) {
                    continue;
                }
                let shifted = prove_range(&mut trie, &perturbed, limit, budget).unwrap();
                assert_eq!(
                    shifted.leaves, slice.leaves,
                    "case {name} perturbed [{first}, {last}]"
                );
                assert_eq!(
                    check(root, &perturbed, &shifted),
                    Ok(expect_more),
                    "case {name} perturbed [{first}, {last}]"
                );
            }
        }
    }
}

/// Random embedding-shaped workloads, against the bulk builder as oracle and
/// against the client download loop's own invariant.
#[test]
fn random_workloads_agree_with_the_bulk_builder() {
    let mut rng = SplitMix64::new(8297);
    for round in 0..20u64 {
        let leaves = random_leaves(&mut rng, 4 + (round as usize % 37));

        // Two independent constructions of the same tree. The bulk fold is
        // the oracle a range proof's recomputation is a partial re-run of, so
        // agreement here is what makes the rest of the round meaningful.
        let mut trie = built(&leaves);
        let bulk = BinaryTrie::from_sorted_leaves(
            Box::new(InMemoryBinaryTrieDB::new_empty()),
            leaves.clone(),
        )
        .unwrap();
        let root = trie.root();
        assert_eq!(root, bulk.root(), "round {round}");

        // The client loop in miniature: chained slices under a small budget,
        // each verified on its own, resuming past the last key it saw.
        let budget = 1 + (rng.next() as usize % 4);
        let mut downloaded: Vec<(Vec<u8>, [u8; 32])> = Vec::new();
        let mut origin: Vec<u8> = Vec::new();
        loop {
            let slice = prove_range(&mut trie, &origin, &[0xff; 66], budget).unwrap();
            let has_more = check(root, &origin, &slice).unwrap_or_else(|error| {
                panic!("round {round} slice from {origin:?} rejected: {error}")
            });
            let last = slice.leaves.last().expect("the progress rule").0.clone();
            downloaded.extend(slice.leaves);
            if !has_more {
                break;
            }
            origin = increment_key(&last).expect("a key past the last cannot be maximal");
        }
        assert_eq!(downloaded, leaves, "round {round} did not reassemble");
    }
}

/// A byte a hostile server could change, changed, one at a time.
#[test]
fn adversarial_tampering_is_rejected() {
    let (name, leaves, root) = mid_size_case();
    let mut trie = built(&leaves);

    // A range through the middle of the tree, so both walks have real depth
    // and both sides carry subtrees.
    let (first, last) = (leaves.len() / 4, leaves.len() / 2);
    let origin = leaves[first].0.clone();
    let slice = prove_range(&mut trie, &origin, &leaves[last].0, last - first + 1).unwrap();
    assert_eq!(check(root, &origin, &slice), Ok(true), "case {name}");
    assert!(
        slice.left_proof.len() > 1 && slice.right_proof.len() > 1 && slice.leaves.len() > 3,
        "the range must have real depth on both sides for this to grind anything"
    );

    // Every single-byte flip in either boundary walk.
    for (side, proof) in [(0, &slice.left_proof), (1, &slice.right_proof)] {
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
                    "side {side} node {node} byte {byte} accepted"
                );
            }
        }
    }

    // Every single-byte flip in a leaf key or value.
    for index in 0..slice.leaves.len() {
        for byte in 0..slice.leaves[index].0.len() {
            let mut altered = slice.leaves.clone();
            altered[index].0[byte] ^= 1;
            altered.sort();
            assert!(
                verify_range(
                    root,
                    &origin,
                    &altered,
                    &slice.left_proof,
                    &slice.right_proof
                )
                .is_err(),
                "leaf {index} key byte {byte} accepted"
            );
        }
        let mut altered = slice.leaves.clone();
        altered[index].1[0] ^= 1;
        assert!(
            verify_range(
                root,
                &origin,
                &altered,
                &slice.left_proof,
                &slice.right_proof
            )
            .is_err(),
            "leaf {index} value accepted"
        );
    }
}

/// Whole-response lies, each with the response otherwise intact.
#[test]
fn adversarial_responses_are_rejected() {
    let (_, leaves, root) = mid_size_case();
    let mut trie = built(&leaves);
    let (first, last) = (leaves.len() / 4, leaves.len() / 2);
    let origin = leaves[first].0.clone();
    let slice = prove_range(&mut trie, &origin, &leaves[last].0, last - first + 1).unwrap();
    assert!(slice.leaves.len() > 3, "the loops below need an interior");

    // Gap smuggling: a leaf quietly dropped from the middle, boundaries
    // untouched. This is the attack range proofs exist for.
    for dropped in 1..slice.leaves.len() - 1 {
        let mut gapped = slice.leaves.clone();
        gapped.remove(dropped);
        assert_eq!(
            verify_range(
                root,
                &origin,
                &gapped,
                &slice.left_proof,
                &slice.right_proof
            ),
            Err(RangeProofError::RootMismatch),
            "dropping leaf {dropped}"
        );
    }

    // A leaf invented between two real ones.
    let mut injected = slice.leaves.clone();
    let mut invented = slice.leaves[1].0.clone();
    *invented.last_mut().unwrap() = invented.last().unwrap().wrapping_add(1);
    injected.insert(2, (invented, [0x5a; 32]));
    injected.sort();
    assert!(
        verify_range(
            root,
            &origin,
            &injected,
            &slice.left_proof,
            &slice.right_proof
        )
        .is_err()
    );

    // Reordered leaves, and a leaf reaching back before the origin.
    let mut reordered = slice.leaves.clone();
    reordered.swap(1, 2);
    assert_eq!(
        verify_range(
            root,
            &origin,
            &reordered,
            &slice.left_proof,
            &slice.right_proof
        ),
        Err(RangeProofError::UnsortedLeaves)
    );
    let mut reaching_back = slice.leaves.clone();
    reaching_back.insert(0, leaves[first - 1].clone());
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

    // Boundary walks swapped, and each truncated by its last node.
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
    assert_eq!(
        verify_range(
            root,
            &origin,
            &slice.leaves,
            &slice.left_proof[..slice.left_proof.len() - 1],
            &slice.right_proof
        ),
        Err(RangeProofError::Proof(ProofError::Truncated))
    );
    assert_eq!(
        verify_range(
            root,
            &origin,
            &slice.leaves,
            &slice.left_proof,
            &slice.right_proof[..slice.right_proof.len() - 1]
        ),
        Err(RangeProofError::Proof(ProofError::Truncated))
    );

    // A right walk of a different key the tree really holds: genuine nodes,
    // wrong path.
    let elsewhere = trie.prove_walk(&leaves[last + 2].0).unwrap();
    assert!(verify_range(root, &origin, &slice.leaves, &slice.left_proof, &elsewhere).is_err());

    // A last leaf the tree does not hold, with an honest exclusion walk of
    // it. The walk verifies — it is a real walk — and only the terminal
    // check notices that the leaf it names was invented.
    let absent = increment_key(&slice.leaves.last().unwrap().0).expect("not the maximal key");
    assert!(
        !leaves.iter().any(|(key, _)| key == &absent),
        "the successor of the last returned key must not itself be a key"
    );
    let mut padded = slice.leaves.clone();
    padded.push((absent.clone(), [0x11; 32]));
    assert_eq!(
        verify_range(
            root,
            &origin,
            &padded,
            &slice.left_proof,
            &trie.prove_walk(&absent).unwrap()
        ),
        Err(RangeProofError::RightProofMismatch)
    );
}

/// Emptiness forged where the tree plainly holds keys, at every origin the
/// vector tries offer.
#[test]
fn adversarial_forged_emptiness_is_rejected() {
    for (name, leaves, root) in vector_tries() {
        if leaves.is_empty() {
            continue;
        }
        let mut trie = built(&leaves);
        for (index, (key, _)) in leaves.iter().enumerate() {
            let slice = prove_range(&mut trie, key, &[0xff; 66], 100).unwrap();
            // The honest answer from here is non-empty, so the same walk with
            // the leaves stripped is a lie the walk itself refutes.
            assert!(!slice.leaves.is_empty());
            assert_eq!(
                verify_range(root, key, &[], &slice.left_proof, &[]),
                Err(RangeProofError::MissingLeaves),
                "case {name} origin {index}"
            );
        }

        // Past the greatest key, emptiness is the truth and must verify.
        let past = leaves
            .last()
            .and_then(|(key, _)| increment_key(key))
            .expect("no vector key is maximal");
        let honest = prove_range(&mut trie, &past, &[0xff; 66], 100).unwrap();
        assert!(honest.leaves.is_empty(), "case {name}");
        assert_eq!(check(root, &past, &honest), Ok(false), "case {name}");
    }
}

/// A state that is entirely self-consistent, served for the wrong root.
///
/// Nothing inside the response is wrong: the leaves are real, the walks are
/// real, they agree with each other. Only the root the client came with is
/// different — and that is the one thing a client actually trusts.
#[test]
fn adversarial_wrong_but_consistent_state_is_rejected() {
    let cases: Vec<_> = vector_tries()
        .into_iter()
        .filter(|(_, leaves, _)| !leaves.is_empty())
        .collect();

    for (name, leaves, _) in &cases {
        let mut trie = built(leaves);
        let slice = prove_range(&mut trie, &[], &[0xff; 66], 100).unwrap();
        for (other_name, _, other_root) in &cases {
            if other_name == name {
                continue;
            }
            assert!(
                verify_range(
                    *other_root,
                    &[],
                    &slice.leaves,
                    &slice.left_proof,
                    &slice.right_proof
                )
                .is_err(),
                "{name}'s range verified against {other_name}'s root"
            );
        }
    }
}

#[test]
fn empty_and_boundary_origins() {
    let (_, leaves, root) = mid_size_case();
    let mut trie = built(&leaves);

    // From an all-zero origin, from the empty origin, and from an all-0xff
    // origin, which `increment_key` reports has no successor at all.
    for origin in [vec![], vec![0x00; 34], vec![0xff; 66]] {
        let slice = prove_range(&mut trie, &origin, &[0xff; 66], 100).unwrap();
        let has_more = check(root, &origin, &slice).expect("verifies");
        if origin == vec![0xff; 66] {
            assert!(slice.leaves.is_empty() && !has_more);
            assert_eq!(increment_key(&origin), None);
        } else {
            assert_eq!(slice.leaves[0], leaves[0]);
        }
    }

    // A two-leaf tree splitting at bit 0, where the left walk's terminal *is*
    // the root branch and the whole range rests on one item.
    let two: Vec<(Vec<u8>, [u8; 32])> = vec![
        (vec![0x00; 34], [1u8; 32]),
        ([&[0x80u8][..], &[0x00; 33]].concat(), [2u8; 32]),
    ];
    let mut split = built(&two);
    let root = split.root();
    let mut origin = vec![0x40];
    origin.extend_from_slice(&[0x00; 33]);
    let slice = prove_range(&mut split, &origin, &[0xff; 34], 100).unwrap();
    assert_eq!(slice.leaves, two[1..]);
    assert_eq!(check(root, &origin, &slice), Ok(false));
}

fn mid_size_case() -> VectorTrie {
    vector_tries()
        .into_iter()
        .max_by_key(|(_, leaves, _)| leaves.len())
        .expect("the fixture has cases")
}

/// Keys shaped like the state embedding: a zone byte, a stem drawn from a
/// small alphabet so subtrees really share prefixes, and a sub-index.
///
/// The two zones give the two key lengths, and the differing zone byte is what
/// keeps a 34-byte key from being a bit-prefix of a 66-byte one.
fn random_leaves(rng: &mut SplitMix64, count: usize) -> Vec<(Vec<u8>, [u8; 32])> {
    const ALPHABET: [u8; 6] = [0x00, 0x01, 0x7f, 0x80, 0xfe, 0xff];
    let mut leaves: BTreeMap<Vec<u8>, [u8; 32]> = BTreeMap::new();
    while leaves.len() < count {
        let overflow = rng.next().is_multiple_of(3);
        let (zone, len) = if overflow { (0xffu8, 66) } else { (0x00u8, 34) };
        let mut key = vec![zone];
        while key.len() < len {
            key.push(ALPHABET[(rng.next() % ALPHABET.len() as u64) as usize]);
        }
        let mut value = [0u8; 32];
        value[..8].copy_from_slice(&rng.next().to_be_bytes());
        leaves.insert(key, value);
    }
    leaves.into_iter().collect()
}

/// SplitMix64, so the workloads are reproducible without a `rand` dependency.
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}
