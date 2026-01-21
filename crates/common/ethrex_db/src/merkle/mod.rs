//! State root hash computation.
//!
//! This module implements Merkle tree computation for calculating
//! Ethereum state root hashes, including RLP encoding and Keccak hashing.

mod bloom;
mod node;
mod rlp_encode;
mod trie;

#[cfg(test)]
mod tests;

pub use bloom::BloomFilter;
pub use node::{keccak256, ChildRef, Node, NodeType, EMPTY_ROOT, HASH_SIZE};
pub use rlp_encode::RlpEncoder;
pub use trie::{MerkleTrie, TrieError};
