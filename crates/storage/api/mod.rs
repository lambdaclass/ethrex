//! # Storage Backend API
//!
//! This module provides a thin, minimal interface for storage backends:
//!
//! - Thin: Minimal set of operations that databases must provide
//! - Simple: Avoids type-system complexity and focuses on core functionality
//!
//! Rather than implementing business logic in each database backend, this API
//! provides low-level primitives that higher-level code can build upon.
//! This eliminates code duplication and makes adding new database backends trivial.
//!
//! The API differentiates between three types of database access:
//!
//! - Read views ([`StorageReadView`]): read-only views of the database,
//!   with no atomicity guarantees between operations.
//! - Write batches ([`StorageWriteBatch`]): write batch functionality, with
//!   atomicity guarantees at commit time.
//! - Locked views ([`StorageLockedView`]): read-only views of a point in time (snapshots), right now it's
//!   only used during snap-sync.

use crate::error::StoreError;
use std::{fmt::Debug, path::Path, sync::Arc};

pub mod tables;

/// Type alias for the result of a prefix iterator.
pub type PrefixResult = Result<(Box<[u8]>, Box<[u8]>), StoreError>;

/// This trait provides a minimal set of operations required from a database backend.
/// Implementations should focus on providing efficient access to the underlying storage
/// without implementing business logic.
pub trait StorageBackend: Debug + Send + Sync {
    /// Removes all data from the specified table.
    fn clear_table(&self, table: &'static str) -> Result<(), StoreError>;

    /// Opens a new read view.
    fn begin_read(&self) -> Result<Arc<dyn StorageReadView>, StoreError>;

    /// Creates a new write batch.
    fn begin_write(&self) -> Result<Box<dyn StorageWriteBatch + 'static>, StoreError>;

    /// Creates a locked snapshot for a specific table.
    ///
    /// This provides a persistent read-only view of a single table, optimized
    /// for batch read operations. The snapshot remains valid until dropped.
    fn begin_locked(
        &self,
        table_name: &'static str,
    ) -> Result<Box<dyn StorageLockedView + 'static>, StoreError>;

    // TODO: remove this and provide historic data via diff-layers
    /// Creates a checkpoint of the current database state at the specified path.
    fn create_checkpoint(&self, path: &Path) -> Result<(), StoreError>;

    /// Durably persists all buffered writes to disk, so a subsequent process
    /// start needs no crash recovery. Called on graceful shutdown. Defaults to a
    /// no-op for backends that are already durable or purely in-memory.
    fn flush(&self) -> Result<(), StoreError> {
        Ok(())
    }
}

/// Read-only transaction interface.
/// Provides methods to read data from the database
pub trait StorageReadView: Send + Sync {
    /// Retrieves a value by key from the specified table.
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError>;

    /// Retrieves multiple values by key from the specified table.
    /// Returns results in the same order as the input keys.
    /// Backends that support batched reads (e.g. RocksDB `multi_get_cf`)
    /// should override this for better throughput. Callers should not
    /// assume `multi_get` is asymptotically faster than `get`; on backends
    /// without a batched read primitive (e.g. the in-memory backend) the
    /// default impl below is equivalent to N independent `get` calls.
    fn multi_get(
        &self,
        table: &'static str,
        keys: &[&[u8]],
    ) -> Vec<Result<Option<Vec<u8>>, StoreError>> {
        keys.iter().map(|k| self.get(table, k)).collect()
    }

    /// Returns an iterator over all key-value pairs with the given prefix.
    fn prefix_iterator(
        &self,
        table: &'static str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError>;

    /// Returns the highest key in `table` by lexicographic order, or `None` if the table is
    /// empty. Backends that support reverse iteration (e.g. RocksDB `IteratorMode::End`) should
    /// implement this in O(1).
    fn last_key(&self, table: &'static str) -> Result<Option<Vec<u8>>, StoreError>;
}

/// Write transaction interface.
///
/// Note that this does not provide read access, since we don't currently use that functionality.
///
/// Changes are not persisted until [`commit()`](StorageWriteBatch::commit) is called.
pub trait StorageWriteBatch: Send {
    /// Stores a key-value pair in the specified table.
    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        self.put_batch(table, vec![(key.to_vec(), value.to_vec())])
    }

    /// Stores multiple key-value pairs in the specified table within the transaction.
    fn put_batch(
        &mut self,
        table: &'static str,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError>;

    /// Removes a key-value pair from the specified table.
    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError>;

    /// Removes every key in `[start, end)` from the specified table.
    ///
    /// Half-open range; `end` is exclusive. Equivalent to enumerating each key
    /// in the range and calling [`delete`], but backends with native range-delete
    /// support (e.g. RocksDB's `delete_range_cf`) can implement it more efficiently.
    ///
    /// Lexicographic byte order is used for the range bounds — callers using
    /// numeric keys must encode them in a representation whose lex order matches
    /// numeric order (e.g. `u64::to_be_bytes()`).
    fn delete_range(
        &mut self,
        table: &'static str,
        start: &[u8],
        end: &[u8],
    ) -> Result<(), StoreError>;

    /// Appends a merge operand for the given key in the specified table.
    ///
    /// The actual combine step is deferred — backends with a registered merge
    /// operator (RocksDB) apply it at read or compaction time; backends without
    /// (InMemory) dispatch by table and apply inline.
    ///
    /// Currently used for `TRANSACTION_LOCATIONS`. Calling on a table without
    /// a registered merge function is an error.
    fn merge(&mut self, table: &'static str, key: &[u8], operand: &[u8]) -> Result<(), StoreError>;

    /// Commits all changes made in this transaction.
    fn commit(&mut self) -> Result<(), StoreError>;
}

/// Locked snapshot interface for batch read operations.
/// Provides read-only access to a specific table with a persistent snapshot.
/// This is optimized for scenarios where many reads are performed on the same
/// table, such as trie traversal operations.
/// This is currently only used in snapsync stage.
// TODO: Check if we can remove this trait and use [`StorageReadView`] instead.
pub trait StorageLockedView: Send + Sync {
    /// Retrieves a value by key from the locked table.
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError>;
}
