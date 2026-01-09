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
pub use store::{AccountUpdatesList, EngineType, Store, UpdateBatch, hash_address, hash_key};

/// Store Schema Version, must be updated on any breaking change
/// An upgrade to a newer schema version invalidates currently stored data, requiring a re-sync.
pub const STORE_SCHEMA_VERSION: u64 = 1;

/// Name of the file storing the metadata about the database
pub const STORE_METADATA_FILENAME: &str = "metadata.json";
