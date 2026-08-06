//! CometBFT's RFC-6962 SHA256 Merkle tree (`crypto/merkle`).
//!
//! Used for the validator-set hash and the header hash. `tmhash` is the full
//! 32-byte SHA256. Domain separation: leaves are prefixed with `0x00`, inner
//! nodes with `0x01`. The split point is the largest power of two strictly less
//! than the item count.

use sha2::{Digest, Sha256};

fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for p in parts {
        hasher.update(p);
    }
    hasher.finalize().into()
}

/// `SHA256(0x00 || leaf)`.
fn leaf_hash(leaf: &[u8]) -> [u8; 32] {
    sha256(&[&[0x00], leaf])
}

/// `SHA256(0x01 || left || right)`.
fn inner_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    sha256(&[&[0x01], left, right])
}

/// Largest power of two strictly less than `n` (n >= 2). Matches CometBFT's
/// `getSplitPoint`.
fn split_point(n: usize) -> usize {
    debug_assert!(n >= 2);
    // Largest power of two <= n, then halved if it equals n.
    let mut k = 1usize;
    while k << 1 < n {
        k <<= 1;
    }
    k
}

/// Merkle root over the given byte-slice leaves (`HashFromByteSlices`).
pub fn hash_from_byte_slices(items: &[Vec<u8>]) -> [u8; 32] {
    match items.len() {
        0 => sha256(&[]),
        1 => leaf_hash(&items[0]),
        n => {
            let k = split_point(n);
            let left = hash_from_byte_slices(&items[..k]);
            let right = hash_from_byte_slices(&items[k..]);
            inner_hash(&left, &right)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_sha256_of_empty() {
        assert_eq!(hash_from_byte_slices(&[]), sha256(&[]));
    }

    #[test]
    fn single_leaf() {
        let items = vec![vec![1u8, 2, 3]];
        assert_eq!(hash_from_byte_slices(&items), leaf_hash(&[1, 2, 3]));
    }

    #[test]
    fn two_leaves() {
        let items = vec![vec![0xaa], vec![0xbb]];
        let expected = inner_hash(&leaf_hash(&[0xaa]), &leaf_hash(&[0xbb]));
        assert_eq!(hash_from_byte_slices(&items), expected);
    }

    #[test]
    fn split_point_values() {
        assert_eq!(split_point(2), 1);
        assert_eq!(split_point(3), 2);
        assert_eq!(split_point(4), 2);
        assert_eq!(split_point(5), 4);
        assert_eq!(split_point(7), 4);
        assert_eq!(split_point(8), 4);
        assert_eq!(split_point(9), 8);
    }
}
