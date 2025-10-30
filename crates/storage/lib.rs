// New unified storage interface
pub mod api;
pub mod backend;
pub mod error;
mod layering;
pub mod rlp;
pub mod store;
pub mod trie;
pub mod utils;

pub use layering::apply_prefix;
pub use store::{
    AccountUpdatesList, EngineType, MAX_SNAPSHOT_READS, STATE_TRIE_SEGMENTS, Store, UpdateBatch,
    hash_address, hash_key,
};
