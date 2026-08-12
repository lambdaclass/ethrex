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

    /// Every key-value pair in the locked table whose key is **at or after**
    /// `start`, in ascending lexicographic key order, running to the end of the
    /// table.
    ///
    /// A **seek**, and deliberately not
    /// [`prefix_iterator`](StorageReadView::prefix_iterator). Those two are the
    /// same call shape and mean different things, so read this before reaching
    /// for either:
    ///
    /// - `start` bounds the iteration *below* and not at all above. A `start`
    ///   that happens to be a prefix of some longer key does not stop the scan
    ///   when the keys stop sharing it — `range_from(b"\x01")` over
    ///   `[b"\x01", b"\x01\x01", b"\x02"]` yields all three.
    /// - An empty `start` is the whole table, which is the one input on which
    ///   this and `prefix_iterator` agree.
    /// - A table the backend has never seen is empty, not an error.
    ///
    /// `prefix_iterator` cannot be used as a seek on the strength of its name:
    /// its two implementations disagree. RocksDB's `prefix_iterator_cf` sets
    /// `prefix_same_as_start(true)`, but no prefix extractor is configured on
    /// any column family, so the option has nothing to act on and the iterator
    /// runs to the end of the CF; the in-memory one filters to
    /// `key.starts_with(prefix)`. That divergence is pre-existing — `store.rs`
    /// already depends on the RocksDB behaviour when seeking `RECEIPTS_V2` — and
    /// is left in place. This method is the portable primitive to use instead,
    /// and `api::tests::range_from_conformance` runs the same scenario against
    /// both backends so the two cannot drift again.
    ///
    /// **On a locked view rather than a read view, and that is the point.** An
    /// ordered scan is a long-running read; a concurrent flush or compaction
    /// under an unpinned iterator can move a row across the cursor, so the scan
    /// misses it or returns it twice. A range served to a peer is supposed to be
    /// exactly the tree's content over an interval and is proved against a root
    /// that fails on the smallest gap. The view is pinned when
    /// [`begin_locked`](StorageBackend::begin_locked) is called and stays pinned
    /// until it is dropped, so a caller must hold the view for the whole scan —
    /// the iterator borrows it, which makes that a compile-time obligation
    /// rather than a convention.
    fn range_from<'a>(
        &'a self,
        start: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + 'a>, StoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tables::MISC_VALUES;
    use crate::backend::in_memory::InMemoryBackend;

    /// Rows chosen so that a prefix filter and a seek give different answers:
    /// `[0x01]` is a prefix of `[0x01, 0x01]`, and `[0x02]` shares no prefix
    /// with either.
    fn seed(backend: &dyn StorageBackend) {
        let mut tx = backend.begin_write().expect("write batch");
        tx.put_batch(
            MISC_VALUES,
            vec![
                (vec![0x01], b"a".to_vec()),
                (vec![0x01, 0x01], b"b".to_vec()),
                (vec![0x02], b"c".to_vec()),
                (vec![0x02, 0xff], b"d".to_vec()),
                (vec![0xff], b"e".to_vec()),
            ],
        )
        .expect("put");
        tx.commit().expect("commit");
    }

    fn collect(view: &dyn StorageLockedView, start: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
        view.range_from(start)
            .expect("range_from")
            .map(|entry| {
                let (key, value) = entry.expect("row");
                (key.into_vec(), value.into_vec())
            })
            .collect()
    }

    /// The agreement the two backends must hold to, asserted identically
    /// against each. Every case here is one `prefix_iterator` gets wrong on at
    /// least one backend.
    fn assert_range_from_semantics(backend: &dyn StorageBackend) {
        // Nothing written yet: an untouched table scans empty rather than
        // failing. RocksDB has the CF, the in-memory map has no entry for it,
        // and both must answer the same way.
        let empty_view = backend.begin_locked(MISC_VALUES).expect("locked view");
        assert_eq!(collect(&*empty_view, &[]), Vec::new());
        assert_eq!(collect(&*empty_view, &[0x01]), Vec::new());
        drop(empty_view);

        seed(backend);
        let view = backend.begin_locked(MISC_VALUES).expect("locked view");

        let all = vec![
            (vec![0x01], b"a".to_vec()),
            (vec![0x01, 0x01], b"b".to_vec()),
            (vec![0x02], b"c".to_vec()),
            (vec![0x02, 0xff], b"d".to_vec()),
            (vec![0xff], b"e".to_vec()),
        ];

        // An empty start is the whole table, in ascending key order.
        assert_eq!(collect(&*view, &[]), all);

        // A start equal to a present key includes that key, and does **not**
        // stop at the end of the keys sharing it as a prefix. This is the case
        // `prefix_iterator` gets wrong on the in-memory backend, which would
        // return only the first two rows.
        assert_eq!(collect(&*view, &[0x01]), all);

        // A start between keys lands on the successor.
        assert_eq!(collect(&*view, &[0x01, 0x00]), all[1..].to_vec());
        assert_eq!(collect(&*view, &[0x01, 0x02]), all[2..].to_vec());

        // A start above every key is empty. This is the case `prefix_iterator`
        // gets wrong on RocksDB, which ignores the prefix and returns
        // everything from the seek point — here, nothing — but would return the
        // whole table for a start that sorts before the first key.
        assert_eq!(collect(&*view, &[0xff, 0x00]), Vec::new());

        // A start below every key is the whole table.
        assert_eq!(collect(&*view, &[0x00]), all);

        // A start on the last key is that key alone.
        assert_eq!(collect(&*view, &[0xff]), all[4..].to_vec());
    }

    /// The other half of the agreement, and the half a row-for-row comparison
    /// cannot see: the scan reads the view's own pinned snapshot, not the live
    /// table.
    ///
    /// This is what makes `range_from` a *locked* primitive rather than a
    /// renamed `prefix_iterator`. On RocksDB the two return identical rows —
    /// `prefix_iterator_cf` sets `prefix_same_as_start` on a CF with no prefix
    /// extractor, so it degenerates to exactly this seek — and swapping one for
    /// the other passes every assertion above. What it does not pass is this:
    /// `prefix_iterator_cf` runs on the database handle, so it observes writes
    /// the snapshot predates.
    fn assert_range_from_is_pinned(backend: &dyn StorageBackend) {
        seed(backend);
        let view = backend.begin_locked(MISC_VALUES).expect("locked view");
        let before = collect(&*view, &[]);

        // Insert a row that sorts in the middle of the range, and delete one
        // that is already in it. Both directions matter: an unpinned scan
        // gains the first and loses the second, and a range proof fails on
        // either.
        let mut tx = backend.begin_write().expect("write batch");
        tx.put_batch(MISC_VALUES, vec![(vec![0x01, 0x80], b"late".to_vec())])
            .expect("put");
        tx.delete(MISC_VALUES, &[0x02]).expect("delete");
        tx.commit().expect("commit");

        assert_eq!(
            collect(&*view, &[]),
            before,
            "the locked view must not observe writes made after begin_locked"
        );
        // The rows really did land, so the assertion above is not vacuous.
        let after = backend.begin_locked(MISC_VALUES).expect("locked view");
        let fresh = collect(&*after, &[]);
        assert!(fresh.iter().any(|(key, _)| key.as_slice() == [0x01, 0x80]));
        assert!(!fresh.iter().any(|(key, _)| key.as_slice() == [0x02]));
    }

    #[test]
    fn range_from_conformance_in_memory() {
        let backend = InMemoryBackend::open().expect("in-memory backend");
        assert_range_from_semantics(&backend);
    }

    #[test]
    fn range_from_is_pinned_in_memory() {
        let backend = InMemoryBackend::open().expect("in-memory backend");
        assert_range_from_is_pinned(&backend);
    }

    /// The same assertions against RocksDB. Decision 13 of the binary-flat
    /// plan records that the two backends had already drifted on
    /// `prefix_iterator` precisely because no test ran the same scenario
    /// against both; this is that test for the primitive that replaces it.
    #[cfg(feature = "rocksdb")]
    #[test]
    fn range_from_conformance_rocksdb() {
        use crate::backend::rocksdb::{RocksDBBackend, RocksDBConfig};
        let dir = tempfile::tempdir().expect("tempdir");
        let backend =
            RocksDBBackend::open(dir.path(), RocksDBConfig::default()).expect("rocksdb backend");
        assert_range_from_semantics(&backend);
    }

    #[cfg(feature = "rocksdb")]
    #[test]
    fn range_from_is_pinned_rocksdb() {
        use crate::backend::rocksdb::{RocksDBBackend, RocksDBConfig};
        let dir = tempfile::tempdir().expect("tempdir");
        let backend =
            RocksDBBackend::open(dir.path(), RocksDBConfig::default()).expect("rocksdb backend");
        assert_range_from_is_pinned(&backend);
    }
}
