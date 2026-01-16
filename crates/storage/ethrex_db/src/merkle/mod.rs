//! State root hash computation.
//!
//! This module implements Merkle tree computation for calculating
//! Ethereum state root hashes, including RLP encoding and Keccak hashing.

mod node;
mod trie;
mod rlp_encode;

#[cfg(test)]
mod tests;

pub use node::{Node, NodeType, keccak256, EMPTY_ROOT, HASH_SIZE};
pub use trie::{MerkleTrie, TrieError};
pub use rlp_encode::RlpEncoder;
