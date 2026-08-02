//! EIP-8297 Partitioned Binary Tree.
//!
//! [`trie`] is the raw prefix-free key/value tree committing to its
//! contents with a single BLAKE3 root. [`embedding`] maps Ethereum
//! state (accounts, storage, code) onto tree keys and values.
//!
//! Reference: `ethereum/execution-specs`, `src/ethereum/binary_trie/`.

pub mod embedding;
pub mod error;
pub mod trie;

pub use error::BinaryTrieError;
