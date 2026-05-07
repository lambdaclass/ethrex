//! Cross-check `NativeRollup.sol::_computeL1MessagesRoot` against
//! lambdaworks `compute_merkle_root`.

#![allow(clippy::unwrap_used)]

use ethrex_common::H256;
use ethrex_crypto::keccak::Keccak256;
use ethrex_l2_common::merkle_tree::compute_merkle_root;

/// Structural mirror of `NativeRollup.sol::_computeL1MessagesRoot`.
fn solidity_l1_messages_root_mirror(leaves: &[H256]) -> H256 {
    if leaves.is_empty() {
        return H256::zero();
    }

    let count = leaves.len();
    let mut len = next_power_of_two(count);
    let mut layer = vec![H256::zero(); len];

    let mut last = H256::zero();
    for (i, h) in leaves.iter().enumerate() {
        last = *h;
        layer[i] = last;
    }
    for slot in layer.iter_mut().take(len).skip(count) {
        *slot = last;
    }

    while len > 1 {
        let new_len = len / 2;
        for i in 0..new_len {
            let a = layer[2 * i];
            let b = layer[2 * i + 1];
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            let mut hasher = Keccak256::new();
            hasher.update(lo.as_bytes());
            hasher.update(hi.as_bytes());
            layer[i] = H256::from_slice(&hasher.finalize());
        }
        len = new_len;
    }

    layer[0]
}

fn next_power_of_two(n: usize) -> usize {
    if n <= 1 {
        return 1;
    }
    let mut p = 1;
    while p < n {
        p <<= 1;
    }
    p
}

fn fake_message_hashes(n: usize) -> Vec<H256> {
    (0..n)
        .map(|i| H256::from_low_u64_be(0x00C0_FFEE_0000 + i as u64))
        .collect()
}

#[test]
fn empty_input_yields_zero_root() {
    assert_eq!(solidity_l1_messages_root_mirror(&[]), H256::zero());
    assert_eq!(compute_merkle_root(&[]), H256::zero());
}

#[test]
fn solidity_mirror_matches_lambdaworks_across_sizes() {
    for &n in &[
        1usize, 2, 3, 4, 5, 6, 7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64,
    ] {
        let leaves = fake_message_hashes(n);
        let lambda = compute_merkle_root(&leaves);
        let mirror = solidity_l1_messages_root_mirror(&leaves);
        assert_eq!(
            lambda, mirror,
            "merkle root mismatch for n={n}: lambdaworks={lambda:#x}, solidity-mirror={mirror:#x}"
        );
    }
}

#[test]
fn single_leaf_root_is_the_leaf() {
    // lambdaworks treats `len == 1` as already power-of-two: root == leaf.
    let leaf = H256::from_low_u64_be(0xDEADBEEF);
    assert_eq!(solidity_l1_messages_root_mirror(&[leaf]), leaf);
    assert_eq!(compute_merkle_root(&[leaf]), leaf);
}

#[test]
fn two_leaves_root_is_commutative_keccak() {
    let a = H256::from_low_u64_be(0xAA);
    let b = H256::from_low_u64_be(0xBB);
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let mut hasher = Keccak256::new();
    hasher.update(lo.as_bytes());
    hasher.update(hi.as_bytes());
    let expected = H256::from_slice(&hasher.finalize());

    assert_eq!(solidity_l1_messages_root_mirror(&[a, b]), expected);
    assert_eq!(compute_merkle_root(&[a, b]), expected);
    assert_eq!(compute_merkle_root(&[b, a]), expected);
}

#[test]
fn swapping_non_sibling_leaves_changes_root() {
    let mut leaves = fake_message_hashes(4);
    let r1 = compute_merkle_root(&leaves);
    leaves.swap(0, 2);
    assert_ne!(r1, compute_merkle_root(&leaves));
}
