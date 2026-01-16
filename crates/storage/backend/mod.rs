//! This module contains the implementations of the [`StorageBackend`](crate::api::StorageBackend) trait for our
//! different databases.

/// In memory backend - most useful for testing
pub mod in_memory;
/// RocksDB backend
#[cfg(feature = "rocksdb")]
pub mod rocksdb;
/// PagedDb backend (Paprika-inspired storage)
#[cfg(feature = "paged-db")]
pub mod paged_db;
