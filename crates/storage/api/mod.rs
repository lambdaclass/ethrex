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

    /// Engine-internal counters, for diagnosing storage behaviour on a devnet.
    ///
    /// `None` for backends with no such notion. See [`StorageStats`] for what
    /// is and is not gated.
    fn stats(&self) -> Option<StorageStats> {
        None
    }
}

/// Per-table storage-engine counters.
///
/// A devnet run on 2026-08-08 had to shut the node down, then count SST files
/// and parse the RocksDB `LOG` by hand to get exactly these four numbers. They
/// are cheap RocksDB properties and need no statistics object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableStats {
    /// Table (column-family) name.
    pub table: &'static str,
    /// `rocksdb.estimate-num-keys` — live keys, memtables plus SSTs.
    pub estimated_keys: u64,
    /// `rocksdb.total-sst-files-size` — bytes across *all* SST files, including
    /// versions superseded by a newer write but not yet compacted away.
    pub sst_bytes: u64,
    /// `rocksdb.estimate-live-data-size` — bytes after deduplication, i.e. the
    /// on-disk footprint of the live set. This is the figure to compare between
    /// tables; `sst_bytes` inflates with write amplification.
    pub live_data_bytes: u64,
    /// `rocksdb.size-all-mem-tables` — bytes not yet flushed. On a small devnet
    /// this is where *everything* is: the 2026-08-08 run saw zero flushes, so
    /// no SST existed and neither SST figure above meant anything.
    pub memtable_bytes: u64,
}

/// A snapshot of storage-engine diagnostics.
#[derive(Debug, Clone, Default)]
pub struct StorageStats {
    /// The engine's own counter dump (RocksDB tickers and histograms: bloom
    /// hit/miss, block-cache hit/miss, read and write latency).
    ///
    /// `None` unless statistics were switched on when the store was opened —
    /// collecting them costs roughly 5-10% throughput, so they are off by
    /// default. `--rocksdb.statistics` turns them on.
    ///
    /// Note that RocksDB's block-cache tickers are **DB-global**, not per
    /// column family, so these attribute to the database as a whole.
    pub engine_statistics: Option<String>,
    /// Per-table properties. Always populated — these are free.
    pub tables: Vec<TableStats>,
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

    /// Returns the lowest key in `table` by lexicographic order, or `None` if the table is
    /// empty. Backends that support forward iteration (e.g. RocksDB `IteratorMode::Start`)
    /// should implement this in O(1).
    fn first_key(&self, table: &'static str) -> Result<Option<Vec<u8>>, StoreError>;

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
