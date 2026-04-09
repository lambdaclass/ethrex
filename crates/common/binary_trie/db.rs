/// Persistence backend trait for the binary trie.
///
/// This trait abstracts over the storage engine (RocksDB, in-memory, etc.)
/// so that `NodeStore` and `BinaryTrieState` don't depend on RocksDB directly.
/// The storage crate implements this trait via its `StorageBackend`.
use crate::error::BinaryTrieError;

/// A single write operation in an atomic batch.
///
/// Keys are stored as `Box<[u8]>` (exact-size heap slice) to avoid the extra
/// capacity word of `Vec<u8>` and to allow zero-copy construction from fixed-size
/// arrays (e.g. `[u8; 8]` node keys via `Box::from`).
pub enum WriteOp {
    Put {
        table: &'static str,
        key: Box<[u8]>,
        value: Vec<u8>,
    },
    Delete {
        table: &'static str,
        key: Box<[u8]>,
    },
}

/// Backend for binary trie persistence.
///
/// All operations are scoped to named tables (column families in RocksDB).
/// Table names are always `&'static str` constants from `ethrex_storage::api::tables`.
pub trait TrieBackend: Send + Sync {
    /// Read a single key from the given table.
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, BinaryTrieError>;

    /// Atomically apply a batch of write operations.
    fn write_batch(&self, ops: Vec<WriteOp>) -> Result<(), BinaryTrieError>;

    /// Iterate over all key-value pairs in the given table.
    /// Used for loading storage keys on initialization.
    fn full_iterator(
        &self,
        table: &'static str,
    ) -> Result<Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)>>, BinaryTrieError>;
}
