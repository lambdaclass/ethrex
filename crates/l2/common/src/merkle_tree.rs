// Re-export Merkle tree functions from ethrex-common.
// This module maintains API compatibility for existing l2-common users.
pub use ethrex_common::merkle_tree::{compute_merkle_proof, compute_merkle_root};
