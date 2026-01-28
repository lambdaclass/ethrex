//! This module contains the implementations of the [`StorageBackend`](crate::api::StorageBackend) trait for our
//! different databases.

/// ethrex-db backend - optimized state storage
#[cfg(feature = "ethrex-db")]
pub mod ethrex_db;
/// In memory backend - most useful for testing
pub mod in_memory;
/// RocksDB backend
#[cfg(feature = "rocksdb")]
pub mod rocksdb;
