pub mod in_memory;
#[cfg(feature = "libmdbx")]
pub mod libmdbx;
#[cfg(feature = "rocksdb")]
pub mod rocksdb;
mod trie_adapter;
use crate::error::StoreError;
pub use in_memory::InMemoryBackend;
#[cfg(feature = "libmdbx")]
pub use libmdbx::LibmdbxBackend;
#[cfg(feature = "rocksdb")]
pub use rocksdb::RocksDBBackend;
use std::fmt::Debug;
use std::panic::RefUnwindSafe;
pub use trie_adapter::{StorageBackendLockedTrieDB, StorageBackendTrieDB};

/// Generic storage backend that knows only about bytes and namespaces.
/// This trait abstracts over different database implementations (RocksDB, LibMDBX, InMemory).
#[async_trait::async_trait]
pub trait StorageBackend: Send + Sync + Debug + RefUnwindSafe {
    /// Get a value by key from the specified namespace
    fn get_sync(&self, namespace: &str, key: Vec<u8>) -> Result<Option<Vec<u8>>, StoreError>;

    /// Get a value by key from the specified namespace
    async fn get_async(&self, namespace: &str, key: Vec<u8>)
    -> Result<Option<Vec<u8>>, StoreError>;

    /// Get a value by key from the specified namespace
    async fn get_async_batch(
        &self,
        namespace: &str,
        keys: Vec<Vec<u8>>,
    ) -> Result<Vec<Vec<u8>>, StoreError>;

    /// Put a key-value pair in the specified namespace
    fn put_sync(&self, namespace: &str, key: Vec<u8>, value: Vec<u8>) -> Result<(), StoreError>;

    /// Put a key-value pair in the specified namespace
    async fn put(&self, namespace: &str, key: Vec<u8>, value: Vec<u8>) -> Result<(), StoreError>;

    /// Delete a key from the specified namespace
    async fn delete(&self, namespace: &str, key: Vec<u8>) -> Result<(), StoreError>;

    /// Execute multiple operations atomically
    async fn batch_write(&self, ops: Vec<BatchOp>) -> Result<(), StoreError>;

    /// Get a range of key-value pairs from start_key (inclusive) to end_key (exclusive)
    /// If end_key is None, iterate from start_key to end of namespace
    async fn range(
        &self,
        namespace: &str,
        start_key: Vec<u8>,
        end_key: Option<Vec<u8>>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError>;
}

/// A batch operation for atomic writes
#[derive(Debug, Clone)]
pub enum BatchOp {
    Put {
        namespace: String,
        key: Vec<u8>,
        value: Vec<u8>,
    },
    Delete {
        namespace: String,
        key: Vec<u8>,
    },
}
