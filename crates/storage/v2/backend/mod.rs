mod r#trait;
pub mod in_memory;
pub mod rocksdb;
pub mod libmdbx;

pub use r#trait::{StorageBackend, BatchOp, StorageError};
pub use in_memory::InMemoryBackend;

#[cfg(feature = "rocksdb")]
pub use rocksdb::RocksDBBackend;

#[cfg(feature = "libmdbx")]
pub use libmdbx::LibmdbxBackend;