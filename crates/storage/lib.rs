// New unified storage interface
pub mod api;
pub mod backend;
pub mod engine;
pub mod error;
pub mod rlp;
pub mod store;
pub mod trie;
pub mod utils;

// Re-exports for public API
pub use store::{
    AccountUpdatesList, EngineType, MAX_SNAPSHOT_READS, STATE_TRIE_SEGMENTS, Store, UpdateBatch,
    hash_address, hash_key,
};
