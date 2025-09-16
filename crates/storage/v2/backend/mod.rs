pub mod in_memory;

#[cfg(feature = "libmdbx")]
pub mod libmdbx;

#[cfg(feature = "rocksdb")]
pub mod rocksdb;

mod r#trait;

pub use in_memory::InMemoryBackend;
pub use r#trait::{BatchOp, StorageBackend, StorageError};

#[cfg(feature = "rocksdb")]
pub use rocksdb::RocksDBBackend;

#[cfg(feature = "libmdbx")]
pub use libmdbx::LibmdbxBackend;
