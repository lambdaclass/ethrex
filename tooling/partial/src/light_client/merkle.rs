//! SSZ Merkle branches and signing roots, as the consensus specs define them. `compute_domain`
//! and `signing_root` follow ethlambda's `beacon::signing` to the byte.

use ethrex_common::H256;
use sha2::{Digest, Sha256};

/// `sha256(a || b)`.
pub fn hash_pair(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(a);
    hasher.update(b);
    hasher.finalize().into()
}

/// `floorlog2`.
pub const fn floorlog2(x: u64) -> u32 {
    if x == 0 { 0 } else { 63 - x.leading_zeros() }
}

/// `get_subtree_index`: the position of `gindex` within its layer.
pub const fn subtree_index(gindex: u64) -> u64 {
    gindex ^ (1 << floorlog2(gindex))
}

/// `is_valid_merkle_branch`.
pub fn is_valid_merkle_branch(
    leaf: [u8; 32],
    branch: &[[u8; 32]],
    depth: u32,
    index: u64,
    root: [u8; 32],
) -> bool {
    if branch.len() < depth as usize {
        return false;
    }
    let mut value = leaf;
    for (i, node) in branch.iter().enumerate().take(depth as usize) {
        if (index >> i) & 1 == 1 {
            value = hash_pair(node, &value);
        } else {
            value = hash_pair(&value, node);
        }
    }
    value == root
}

/// `is_valid_normalized_merkle_branch`: a branch of any length whose extra leading nodes
/// are zero (a shorter container's branch brought to a longer one).
pub fn is_valid_normalized_merkle_branch(
    leaf: [u8; 32],
    branch: &[[u8; 32]],
    gindex: u64,
    root: [u8; 32],
) -> bool {
    let depth = floorlog2(gindex) as usize;
    let index = subtree_index(gindex);
    let Some(extra) = branch.len().checked_sub(depth) else {
        return false;
    };
    let Some((leading, rest)) = branch.split_at_checked(extra) else {
        return false;
    };
    if leading.iter().any(|node| *node != [0u8; 32]) {
        return false;
    }
    is_valid_merkle_branch(leaf, rest, depth as u32, index, root)
}

/// `compute_fork_data_root(current_version, genesis_validators_root)`.
pub fn fork_data_root(version: [u8; 4], genesis_validators_root: H256) -> [u8; 32] {
    let mut chunk = [0u8; 32];
    chunk[..4].copy_from_slice(&version);
    hash_pair(&chunk, &genesis_validators_root.0)
}

/// `compute_domain(domain_type, fork_version, genesis_validators_root)`.
pub fn compute_domain(
    domain_type: [u8; 4],
    fork_version: [u8; 4],
    genesis_validators_root: H256,
) -> [u8; 32] {
    let fork_root = fork_data_root(fork_version, genesis_validators_root);
    let mut domain = [0u8; 32];
    domain[..4].copy_from_slice(&domain_type);
    domain[4..].copy_from_slice(&fork_root[..28]);
    domain
}

/// `compute_signing_root(object, domain)` given the object's root.
pub fn signing_root(object_root: H256, domain: [u8; 32]) -> [u8; 32] {
    hash_pair(&object_root.0, &domain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gindex_math() {
        assert_eq!(floorlog2(1), 0);
        assert_eq!(floorlog2(2856), 11);
        assert_eq!(subtree_index(2856), 2856 - 2048);
        assert_eq!(floorlog2(735), 9);
        assert_eq!(floorlog2(2945), 11);
    }

    #[test]
    fn two_leaf_tree_branch() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let root = hash_pair(&a, &b);
        assert!(is_valid_merkle_branch(a, &[b], 1, 0, root));
        assert!(is_valid_merkle_branch(b, &[a], 1, 1, root));
        assert!(!is_valid_merkle_branch(a, &[b], 1, 1, root));
        // gindex 2 = left leaf, 3 = right leaf; a leading zero is tolerated when normalized.
        assert!(is_valid_normalized_merkle_branch(
            a,
            &[[0u8; 32], b],
            2,
            root
        ));
        assert!(!is_valid_normalized_merkle_branch(
            a,
            &[[9u8; 32], b],
            2,
            root
        ));
    }
}
