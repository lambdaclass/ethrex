//! State root hash computation.
//!
//! This module implements Merkle tree computation for calculating
//! Ethereum state root hashes, including RLP encoding and Keccak hashing.

mod node;
mod trie;
mod rlp_encode;
mod bloom;

#[cfg(test)]
mod tests;

pub use node::{Node, NodeType, ChildRef, keccak256, EMPTY_ROOT, HASH_SIZE};
pub use trie::{MerkleTrie, TrieError};
pub use rlp_encode::RlpEncoder;
pub use bloom::BloomFilter;
