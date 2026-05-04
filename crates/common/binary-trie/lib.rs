//! EIP-7864 binary trie implementation for ethrex.
//!
//! Provides a BLAKE3-keyed binary trie for Ethereum state storage, including
//! key derivation, merkelization, proof generation, a persistent state
//! layer (`BinaryTrieState`), and a `StateReader`/`StateCommitter` backend
//! (`BinaryBackend`) that plugs into the `ethrex-state-backend` abstraction.
#![warn(unused_crate_dependencies)]

pub mod backend;
pub mod db;
pub mod error;
pub mod hash;
pub mod key_mapping;
pub mod layer_cache;
pub mod merkle;
pub mod merkleizer;
pub mod node;
pub mod node_store;
pub mod proof;
pub mod state;
pub mod trie;
pub mod witness;

pub use backend::{BinaryBackend, BinaryTrieProvider, EmptyBinaryTrieProvider};
pub use db::{TrieBackend, WriteOp};
pub use error::BinaryTrieError;
pub use hash::{CACHE_TOMBSTONE_TAG, CACHE_VALUE_TAG, EMPTY_BINARY_ROOT};
pub use merkleizer::BinaryMerkleizer;
pub use node_store::{META_NEXT_ID, META_ROOT, META_ROOT_HASH, node_key, serialize_node};
pub use proof::BinaryTrieProof;
pub use state::BinaryTrieState;
pub use trie::BinaryTrie;
pub use witness::BinaryTrieWitness;
