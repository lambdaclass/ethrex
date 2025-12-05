mod api;

#[cfg(feature = "rocksdb")]
mod rlp;
mod store;
pub mod store_db;
mod trie_db;
#[cfg(feature = "rocksdb")]
mod utils;

pub mod error;
pub use store::{
    AccountUpdatesList, EngineType, MAX_SNAPSHOT_READS, STATE_TRIE_SEGMENTS, Store, UpdateBatch,
    hash_address, hash_key,
};
pub use trie_db::layering::apply_prefix;

/// Store Schema Version, must be updated on any breaking change
/// An upgrade to a newer schema version invalidates currently stored data, requiring a re-sync.
pub const STORE_SCHEMA_VERSION: u64 = 1;
