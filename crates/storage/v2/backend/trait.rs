use std::fmt::Debug;
use std::panic::RefUnwindSafe;

use ethrex_rlp::error::RLPDecodeError;

/// Generic storage backend that knows only about bytes and namespaces.
/// This trait abstracts over different database implementations (RocksDB, LibMDBX, InMemory).
#[async_trait::async_trait]
pub trait StorageBackend: Send + Sync + Debug + RefUnwindSafe {
    /// Get a value by key from the specified namespace
    fn get_sync(&self, namespace: &str, key: Vec<u8>) -> Result<Option<Vec<u8>>, StorageError>;

    /// Get a value by key from the specified namespace
    async fn get_async(
        &self,
        namespace: &str,
        key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, StorageError>;

    /// Get a value by key from the specified namespace
    async fn get_async_batch(
        &self,
        namespace: &str,
        keys: Vec<Vec<u8>>,
    ) -> Result<Vec<Vec<u8>>, StorageError>;

    /// Put a key-value pair in the specified namespace
    async fn put(&self, namespace: &str, key: Vec<u8>, value: Vec<u8>) -> Result<(), StorageError>;

    /// Delete a key from the specified namespace
    async fn delete(&self, namespace: &str, key: Vec<u8>) -> Result<(), StorageError>;

    /// Execute multiple operations atomically
    async fn batch_write(&self, ops: Vec<BatchOp>) -> Result<(), StorageError>;

    /// Initialize/ensure a namespace exists (for DBs that require pre-creation)
    async fn init_namespace(&self, namespace: &str) -> Result<(), StorageError>;

    /// Get a range of key-value pairs from start_key (inclusive) to end_key (exclusive)
    /// If end_key is None, iterate from start_key to end of namespace
    async fn range(
        &self,
        namespace: &str,
        start_key: Vec<u8>,
        end_key: Option<&[u8]>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StorageError>;
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

/// Storage backend error types
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Namespace not found: {0}")]
    NamespaceNotFound(String),

    #[error("Custom error: {0}")]
    Custom(String),

    #[error("Error decoding RLP")]
    RLPDecode(#[from] RLPDecodeError),

    #[cfg(feature = "rocksdb")]
    #[error("RocksDB error: {0}")]
    RocksDB(#[from] rocksdb::Error),
}
