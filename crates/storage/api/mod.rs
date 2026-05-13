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

    /// Force in-memory write buffers / memtables to be persisted to durable
    /// storage. Pair with `commit_no_wal` at phase boundaries so a crash after
    /// the phase cannot lose already-completed work.
    ///
    /// Default impl is a no-op (in-memory backends have nothing to flush).
    fn flush(&self) -> Result<(), StoreError> {
        Ok(())
    }
}

/// Read-only transaction interface.
/// Provides methods to read data from the database
pub trait StorageReadView: Send + Sync {
    /// Retrieves a value by key from the specified table.
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError>;

    /// Returns an iterator over all key-value pairs with the given prefix.
    fn prefix_iterator(
        &self,
        table: &'static str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError>;
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

    /// Commits all changes made in this transaction.
    fn commit(&mut self) -> Result<(), StoreError>;

    /// Commits all changes made in this transaction WITHOUT writing to the
    /// write-ahead log. Use ONLY for resumable bulk-insert workloads (e.g.,
    /// snap sync), where crash recovery is handled by re-running the phase
    /// rather than replaying the WAL.
    ///
    /// The caller MUST pair this with `StorageBackend::flush()` at phase
    /// boundaries to ensure committed data is durably on disk before the
    /// phase is declared complete.
    ///
    /// Default impl falls back to a regular `commit()` so backends that don't
    /// differentiate (e.g., in-memory) keep working unchanged.
    fn commit_no_wal(&mut self) -> Result<(), StoreError> {
        self.commit()
    }
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
