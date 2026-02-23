//! OpenZeppelin-compatible Merkle tree implementation using commutative Keccak256 hashing.
//!
//! This module provides functions to compute Merkle roots and proofs that are compatible with
//! OpenZeppelin's MerkleProof.sol contract. The commutative property (H(a, b) == H(b, a)) is
//! achieved by sorting inputs before hashing.
//!
//! See: https://docs.openzeppelin.com/contracts/5.x/api/utils#MerkleProof

use crate::H256;
use ethrex_crypto::keccak::keccak_hash;

/// Compute a Merkle root using commutative Keccak256 hashing (OpenZeppelin-compatible).
///
/// Commutative hashing ensures H(a, b) == H(b, a), which is required for
/// compatibility with OpenZeppelin's MerkleProof.verify().
///
/// See: https://docs.openzeppelin.com/contracts/5.x/api/utils#MerkleProof
pub fn compute_merkle_root(hashes: &[H256]) -> H256 {
    match hashes {
        [] => H256::zero(),
        [single] => *single,
        _ => {
            let mut current_level: Vec<[u8; 32]> = hashes.iter().map(|h| h.0).collect();
            while current_level.len() > 1 {
                current_level = merkle_next_level(&current_level);
            }
            current_level
                .first()
                .map(|h| H256::from(*h))
                .unwrap_or_default()
        }
    }
}

/// Compute a Merkle proof for the leaf at `index`.
///
/// Returns the sibling hashes from leaf to root, suitable for OpenZeppelin's
/// MerkleProof.verify().
pub fn compute_merkle_proof(hashes: &[H256], index: usize) -> Vec<H256> {
    if hashes.len() <= 1 {
        return vec![];
    }

    let mut current_level: Vec<[u8; 32]> = hashes.iter().map(|h| h.0).collect();
    let mut proof = Vec::new();
    let mut idx = index;

    while current_level.len() > 1 {
        // Add sibling to proof if it exists
        let sibling_idx = if idx.is_multiple_of(2) {
            idx.wrapping_add(1)
        } else {
            idx.wrapping_sub(1)
        };
        if let Some(sibling) = current_level.get(sibling_idx) {
            proof.push(H256::from(*sibling));
        }

        current_level = merkle_next_level(&current_level);
        idx /= 2;
    }

    proof
}

/// Build the next level of a Merkle tree from the current level.
///
/// Pairs adjacent elements and hashes them. If there's an odd element,
/// it's promoted to the next level unchanged.
fn merkle_next_level(current_level: &[[u8; 32]]) -> Vec<[u8; 32]> {
    let mut next_level = Vec::new();
    for pair in current_level.chunks(2) {
        match pair {
            [left, right] => next_level.push(commutative_hash(left, right)),
            [single] => next_level.push(*single),
            _ => {}
        }
    }
    next_level
}

/// Commutative Keccak256 hash: H(a, b) == H(b, a).
///
/// Sorts inputs so the smaller value comes first, matching OpenZeppelin's
/// `_hashPair` in MerkleProof.sol.
fn commutative_hash(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut data = [0u8; 64];
    if a <= b {
        data[..32].copy_from_slice(a);
        data[32..].copy_from_slice(b);
    } else {
        data[..32].copy_from_slice(b);
        data[32..].copy_from_slice(a);
    }
    keccak_hash(data)
}
