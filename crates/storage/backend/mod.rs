//! This module contains the implementations of the [`StorageBackend`](crate::api::StorageBackend) trait for our
//! different databases.

/// In memory backend - most useful for testing
pub mod in_memory;
/// Libmdbx backend
#[cfg(feature = "libmdbx")]
pub mod libmdbx;
/// RocksDB backend
#[cfg(feature = "rocksdb")]
pub mod rocksdb;
