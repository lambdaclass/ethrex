use crate::api::tables::{
    ACCOUNT_CODES, ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, BINARY_FLATKEYVALUE,
    BINARY_TRIE_NODES, BLOCK_NUMBERS, BODIES, CANONICAL_BLOCK_HASHES, FULLSYNC_HEADERS, HEADERS,
    RECEIPTS_V2, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES, TRANSACTION_LOCATIONS,
};
use crate::api::{
    PrefixResult, StorageBackend, StorageLockedView, StorageReadView, StorageStats,
    StorageWriteBatch, TableStats, tables::TABLES,
};
use crate::error::StoreError;
use rocksdb::DBWithThreadMode;
use rocksdb::checkpoint::Checkpoint;
use rocksdb::statistics::StatsLevel;
use rocksdb::{
    BlockBasedOptions, Cache, ColumnFamilyDescriptor, MergeOperands, MultiThreaded, Options,
    SnapshotWithThreadMode, WriteBatch, properties,
};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

use crate::store::tx_locations_merge;

/// Adapter wrapping `tx_locations_merge` to match RocksDB's expected signature.
fn tx_locations_merge_op(
    _new_key: &[u8],
    existing: Option<&[u8]>,
    operands: &MergeOperands,
) -> Option<Vec<u8>> {
    tx_locations_merge(existing, operands)
}

/// The subset of [`crate::store::StoreConfig`] the RocksDB backend consumes.
///
/// A struct rather than positional arguments so that `open(path, 12 << 30, false)`
/// can't happen at the five call sites.
#[derive(Debug, Clone, Copy)]
pub struct RocksDBConfig {
    /// Size in bytes of the LRU block cache shared by every column family.
    pub block_cache_size: usize,
    /// Install a RocksDB `Statistics` object, making tickers and histograms
    /// readable through [`RocksDBBackend::stats`].
    ///
    /// Off by default: RocksDB documents statistics collection as costing
    /// roughly 5-10% throughput, which is not a price to pay on every node for
    /// numbers only a diagnostic run reads.
    pub enable_statistics: bool,
}

impl Default for RocksDBConfig {
    fn default() -> Self {
        Self {
            block_cache_size: crate::store::DEFAULT_ROCKSDB_BLOCK_CACHE_SIZE_BYTES,
            enable_statistics: false,
        }
    }
}

/// RocksDB backend
pub struct RocksDBBackend {
    /// Optimistric transaction database
    db: Arc<DBWithThreadMode<MultiThreaded>>,
    /// The `Options` the DB was opened with, retained **only** when statistics
    /// are enabled.
    ///
    /// `enable_statistics()` installs a `shared_ptr<Statistics>` on the options,
    /// and the open DB holds that same pointer — so reading counters means
    /// reading them back off this object. Dropping it here would leave no handle
    /// to the live statistics. `None` when statistics are off, which is what
    /// makes [`RocksDBBackend::stats`] report them absent rather than empty.
    stats_opts: Option<Options>,
}

// `rocksdb::Options` has no `Debug`, so the derive can't be used. The db handle
// is the only field worth printing anyway.
impl std::fmt::Debug for RocksDBBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RocksDBBackend")
            .field("db", &self.db)
            .field("statistics_enabled", &self.stats_opts.is_some())
            .finish()
    }
}

impl RocksDBBackend {
    pub fn open(path: impl AsRef<Path>, config: RocksDBConfig) -> Result<Self, StoreError> {
        let block_cache_size = config.block_cache_size;
        // Rocksdb optimizations options
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        opts.set_max_open_files(-1);
        opts.set_max_file_opening_threads(16);

        opts.set_max_background_jobs(8);

        opts.set_level_zero_file_num_compaction_trigger(2);
        opts.set_level_zero_slowdown_writes_trigger(10);
        opts.set_level_zero_stop_writes_trigger(16);
        opts.set_target_file_size_base(512 * 1024 * 1024); // 512MB
        opts.set_max_bytes_for_level_base(2 * 1024 * 1024 * 1024); // 2GB L1
        opts.set_max_bytes_for_level_multiplier(10.0);
        opts.set_level_compaction_dynamic_level_bytes(true);

        opts.set_db_write_buffer_size(1024 * 1024 * 1024); // 1GB
        opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
        opts.set_max_write_buffer_number(4);
        opts.set_min_write_buffer_number_to_merge(2);

        opts.set_wal_recovery_mode(rocksdb::DBRecoveryMode::PointInTime);
        opts.set_max_total_wal_size(2 * 1024 * 1024 * 1024); // 2GB
        opts.set_wal_bytes_per_sync(32 * 1024 * 1024); // 32MB
        opts.set_bytes_per_sync(32 * 1024 * 1024); // 32MB
        opts.set_use_fsync(false); // fdatasync

        opts.set_enable_pipelined_write(true);
        opts.set_allow_concurrent_memtable_write(true);
        opts.set_enable_write_thread_adaptive_yield(true);
        opts.set_compaction_readahead_size(4 * 1024 * 1024); // 4MB
        opts.set_advise_random_on_open(false);
        opts.set_compression_type(rocksdb::DBCompressionType::None);

        if config.enable_statistics {
            opts.enable_statistics();
            // `ExceptDetailedTimers` keeps every ticker (bloom hit/miss,
            // block-cache hit/miss, bytes read/written) and the read/write
            // latency histograms, while skipping the counters that must take a
            // clock reading inside a mutex — those are the ones that hurt
            // scalability under concurrent writes.
            opts.set_statistics_level(StatsLevel::ExceptDetailedTimers);
        }

        let compressible_tables = [
            BLOCK_NUMBERS,
            HEADERS,
            BODIES,
            RECEIPTS_V2,
            TRANSACTION_LOCATIONS,
            FULLSYNC_HEADERS,
        ];

        // Open all column families
        let existing_cfs = DBWithThreadMode::<MultiThreaded>::list_cf(&opts, path.as_ref())
            .unwrap_or_else(|_| vec!["default".to_string()]);

        let mut all_cfs_to_open = HashSet::new();
        all_cfs_to_open.extend(existing_cfs.iter().cloned());
        all_cfs_to_open.extend(TABLES.iter().map(|table| table.to_string()));

        // Shared block cache for all column families. With
        // `cache_index_and_filter_blocks(true)` below, this cache holds both data blocks
        // and the index/bloom-filter blocks needed to look them up, so its size is the
        // effective ceiling on RocksDB's resident memory footprint. The caller chooses
        // the size (see the `--rocksdb.block-cache-size` CLI flag); a value that is too
        // small relative to the filter + working-set size will degrade block-import
        // throughput (filter blocks displace data blocks, EVM reads spill to disk).
        //
        // Stays shared, deliberately. Giving the trie-node CFs their own cache
        // was considered as a way to make cache behaviour attributable after
        // the 2026-08-08 devnet could not attribute any of it, and rejected:
        //
        //  - It would not actually attribute anything. RocksDB's block-cache
        //    tickers (`rocksdb.block.cache.hit`/`.miss`, and the index/filter/
        //    data variants) are recorded on the *DB's* `Statistics` object, not
        //    per `Cache`. A second cache yields per-cache *occupancy*
        //    (`Cache::get_usage`), never a per-CF hit rate — which is the
        //    number that was wanted.
        //  - It would break the memory ceiling. The single cache is what makes
        //    `--rocksdb.block-cache-size` a bound rather than a suggestion;
        //    two caches means the operator's one number bounds neither, and
        //    the split ratio is a policy choice no measurement supports. The
        //    mainnet sweep behind the 12 GiB default found ~8 GiB is already
        //    the floor where the filter set thrashes, so mis-splitting it
        //    regresses a measured workload to instrument an unmeasured one.
        let block_cache = Cache::new_lru_cache(block_cache_size);

        // Configures a CF's block-based table to keep its index and bloom-filter blocks
        // inside the shared (bounded) block cache rather than pinning them per open file.
        //
        // With `max_open_files(-1)` every SST stays open, and RocksDB's default
        // (`cache_index_and_filter_blocks = false`) pins each file's index + filter blocks
        // in heap for the lifetime of the reader. On a large state DB this grows without
        // bound with the number of SST files (on a 490 GB mainnet DB the pinned filters
        // alone reached ~6 GB). Caching them instead bounds total table memory to the block
        // cache size; pinning L0 keeps the hottest level resident to avoid a read-latency cliff.
        let configure_block_cache = |block_opts: &mut BlockBasedOptions| {
            block_opts.set_block_cache(&block_cache);
            block_opts.set_cache_index_and_filter_blocks(true);
            block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
        };

        // Gives a CF a *working* memtable bloom filter.
        //
        // `memtable_prefix_bloom_size_ratio` only sizes the filter; it does not
        // ask for one. RocksDB allocates the memtable bloom only when a prefix
        // extractor is configured **or** `memtable_whole_key_filtering` is on,
        // and this backend has never set either — so the 0.2 ratio that has sat
        // on the MPT trie/flat CFs since they were tuned (and was inherited by
        // the binary tables) built nothing at all. Confirmed from the applied
        // `OPTIONS` file of a devnet on 2026-08-08: `prefix_extractor=nullptr`
        // and `memtable_whole_key_filtering=false` on every one of them.
        //
        // Whole-key filtering rather than a prefix extractor because these are
        // whole-key point-lookup tables: trie nodes are keyed by a complete
        // path/hash and the flat mirror by a complete account/slot key, and
        // every read is an exact `get`. There is no meaningful key prefix to
        // extract, and inventing one would also silently change iterator
        // semantics (`prefix_iterator_cf` is used on these CFs).
        //
        // This matters most for the binary trie, whose ~33-node descent issues
        // ~5x the point lookups of the MPT's ~7; each miss that the bloom
        // rejects is a skiplist walk avoided. It only binds while data is still
        // in the memtable — the SST-level bloom above covers it after a flush.
        let configure_memtable_bloom = |cf_opts: &mut Options| {
            cf_opts.set_memtable_whole_key_filtering(true);
            // Sizes the filter as a fraction of the write buffer. Keeping 0.2
            // preserves the (previously inert) intent: 512MB * 0.2 of bloom bits.
            cf_opts.set_memtable_prefix_bloom_ratio(0.2);
        };

        let mut cf_descriptors = Vec::new();
        for cf_name in &all_cfs_to_open {
            let mut cf_opts = Options::default();

            cf_opts.set_level_zero_file_num_compaction_trigger(4);
            cf_opts.set_level_zero_slowdown_writes_trigger(20);
            cf_opts.set_level_zero_stop_writes_trigger(36);

            if compressible_tables.contains(&cf_name.as_str()) {
                cf_opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
            } else {
                cf_opts.set_compression_type(rocksdb::DBCompressionType::None);
            }

            match cf_name.as_str() {
                HEADERS | BODIES => {
                    cf_opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
                    cf_opts.set_max_write_buffer_number(4);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(32 * 1024); // 32KB blocks
                    configure_block_cache(&mut block_opts);
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                CANONICAL_BLOCK_HASHES | BLOCK_NUMBERS => {
                    cf_opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(128 * 1024 * 1024); // 128MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024); // 16KB
                    block_opts.set_bloom_filter(10.0, false);
                    configure_block_cache(&mut block_opts);
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                TRANSACTION_LOCATIONS => {
                    cf_opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(128 * 1024 * 1024); // 128MB

                    // The write path uses merge_cf instead of read-modify-write,
                    // so the per-tx negative get is gone. The merge operator
                    // folds (block_number, block_hash, index) operands into the
                    // Vec value on read/compaction.
                    cf_opts.set_merge_operator_associative(
                        "tx_locations_merge",
                        tx_locations_merge_op,
                    );

                    // No bloom filter, intentionally. Bloom only accelerates
                    // negative point lookups, and with the merge operator the
                    // hot write path no longer does per-tx gets. The only
                    // remaining negative reads are user `eth_getTransactionByHash`
                    // on missing hashes — rare and not worth the filter's memory
                    // + the implicit "perf depends on this config" coupling.
                    // (Benchmarked: bloom didn't help the RMW variant either,
                    // since deep-level coverage lags and the memtable traversal
                    // floor is unaffected — see PR #6737.)
                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024); // 16KB
                    // Bound this CF's index blocks in the shared cache too (no bloom
                    // here, but index still grows with SST count if pinned in heap).
                    configure_block_cache(&mut block_opts);
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                // The EIP-8297 tables get the MPT's trie tuning, not the
                // catch-all default. They had been falling into the `_` arm
                // below -- 64MB buffers, three of them, and **no bloom filter**
                // -- while their MPT counterparts get 512MB x 6 and 10 bits per
                // key. That is the worst possible pairing for this trie: a
                // binary radix descent is ~33 node reads deep against the MPT's
                // ~7, so it issues far more point lookups and, without a bloom,
                // each miss pays a full block read.
                //
                // Grouped with the MPT arms rather than given their own because
                // the access shape is identical -- point reads of path-keyed
                // nodes during a descent, and point reads of leaf rows for the
                // flat mirror. Any future divergence should be driven by
                // measurement, not by the tables being new.
                ACCOUNT_TRIE_NODES | STORAGE_TRIE_NODES | BINARY_TRIE_NODES => {
                    cf_opts.set_write_buffer_size(512 * 1024 * 1024); // 512MB
                    cf_opts.set_max_write_buffer_number(6);
                    cf_opts.set_min_write_buffer_number_to_merge(2);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB
                    configure_memtable_bloom(&mut cf_opts);

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024); // 16KB
                    block_opts.set_bloom_filter(10.0, false); // 10 bits per key
                    configure_block_cache(&mut block_opts);
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                ACCOUNT_FLATKEYVALUE | STORAGE_FLATKEYVALUE | BINARY_FLATKEYVALUE => {
                    cf_opts.set_write_buffer_size(512 * 1024 * 1024); // 512MB
                    cf_opts.set_max_write_buffer_number(6);
                    cf_opts.set_min_write_buffer_number_to_merge(2);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB
                    configure_memtable_bloom(&mut cf_opts);

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024); // 16KB
                    block_opts.set_bloom_filter(10.0, false); // 10 bits per key
                    configure_block_cache(&mut block_opts);
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                ACCOUNT_CODES => {
                    cf_opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB

                    cf_opts.set_enable_blob_files(true);
                    // Small bytecodes should go inline (mainly for delegation indicators)
                    cf_opts.set_min_blob_size(32);
                    cf_opts.set_blob_compression_type(rocksdb::DBCompressionType::Lz4);

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(32 * 1024); // 32KB
                    configure_block_cache(&mut block_opts);
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                RECEIPTS_V2 => {
                    cf_opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(32 * 1024); // 32KB
                    configure_block_cache(&mut block_opts);
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                _ => {
                    // Default for other CFs
                    cf_opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(128 * 1024 * 1024); // 128MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024);
                    configure_block_cache(&mut block_opts);
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
            }

            cf_descriptors.push(ColumnFamilyDescriptor::new(cf_name, cf_opts));
        }

        let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
            &opts,
            path.as_ref(),
            cf_descriptors,
        )
        .map_err(|e| StoreError::Custom(format!("Failed to open RocksDB with all CFs: {}", e)))?;

        Ok(Self {
            db: Arc::new(db),
            stats_opts: config.enable_statistics.then_some(opts),
        })
    }

    /// RocksDB's own counter dump: tickers and histograms.
    ///
    /// `None` unless the store was opened with `enable_statistics`. The string
    /// is RocksDB's `Statistics::ToString()`, the same text that
    /// `rocksdb.stats`-style tooling consumes; it is read live off the
    /// `Statistics` object the open DB is still writing to.
    pub fn statistics(&self) -> Option<String> {
        self.stats_opts.as_ref()?.get_statistics()
    }

    /// Per-table size and key-count properties.
    ///
    /// Free — these are computed from metadata RocksDB already maintains, and
    /// need no `Statistics` object, so they are readable on any node.
    /// Tables whose column family is missing are skipped rather than reported
    /// as zero, so a zero here means "empty", not "absent".
    pub fn table_stats(&self) -> Vec<TableStats> {
        TABLES
            .iter()
            .filter_map(|table| {
                let cf = self.db.cf_handle(table)?;
                let property = |name: &properties::PropName| {
                    self.db
                        .property_int_value_cf(&cf, name)
                        .ok()
                        .flatten()
                        .unwrap_or(0)
                };
                Some(TableStats {
                    table,
                    estimated_keys: property(properties::ESTIMATE_NUM_KEYS),
                    sst_bytes: property(properties::TOTAL_SST_FILES_SIZE),
                    live_data_bytes: property(properties::ESTIMATE_LIVE_DATA_SIZE),
                    memtable_bytes: property(properties::SIZE_ALL_MEM_TABLES),
                })
            })
            .collect()
    }

    /// Drops column families that exist on disk but are no longer listed in
    /// `TABLES`. Must be called **after** migrations so that migration code
    /// can still read from legacy CFs (e.g. `receipts` during v1→v2).
    pub fn drop_obsolete_cfs(&self, path: impl AsRef<Path>) {
        let opts = Options::default();
        // Best-effort: if we can't list CFs (e.g. fresh DB), skip cleanup silently.
        let existing_cfs =
            DBWithThreadMode::<MultiThreaded>::list_cf(&opts, path.as_ref()).unwrap_or_default();

        for cf_name in &existing_cfs {
            if cf_name != "default" && !TABLES.contains(&cf_name.as_str()) {
                let _ = self
                    .db
                    .drop_cf(cf_name)
                    .inspect(|_| info!("Dropped obsolete column family '{}'", cf_name))
                    .inspect_err(|e|
                        // Log error but don't fail — the database is still usable
                        warn!("Failed to drop obsolete column family '{}': {}", cf_name, e));
            }
        }
    }
}

impl Drop for RocksDBBackend {
    fn drop(&mut self) {
        // When the last reference to the db is dropped, stop background threads
        // See https://github.com/facebook/rocksdb/issues/11349
        if let Some(db) = Arc::get_mut(&mut self.db) {
            db.cancel_all_background_work(true);
        }
    }
}

impl StorageBackend for RocksDBBackend {
    fn clear_table(&self, table: &'static str) -> Result<(), StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom("Column family not found".to_string()))?;

        let mut iter = self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start);
        let mut batch = WriteBatch::default();

        while let Some(Ok((key, _))) = iter.next() {
            batch.delete_cf(&cf, key);
        }

        self.db
            .write(batch)
            .map_err(|e| StoreError::Custom(format!("RocksDB batch write error: {}", e)))
    }

    fn begin_read(&self) -> Result<Arc<dyn StorageReadView>, StoreError> {
        Ok(Arc::new(RocksDBReadTx {
            db: self.db.clone(),
        }))
    }

    fn begin_write(&self) -> Result<Box<dyn StorageWriteBatch + 'static>, StoreError> {
        let batch = WriteBatch::default();

        Ok(Box::new(RocksDBWriteTx {
            db: self.db.clone(),
            batch,
        }))
    }

    fn begin_locked(
        &self,
        table_name: &'static str,
    ) -> Result<Box<dyn StorageLockedView>, StoreError> {
        let db = Box::leak(Box::new(self.db.clone()));
        let lock = db.snapshot();
        let cf = db
            .cf_handle(table_name)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table_name)))?;

        Ok(Box::new(RocksDBLocked { db, lock, cf }))
    }

    fn create_checkpoint(&self, path: &Path) -> Result<(), StoreError> {
        let checkpoint = Checkpoint::new(&self.db)
            .map_err(|e| StoreError::Custom(format!("Failed to create checkpoint: {e}")))?;

        checkpoint.create_checkpoint(path).map_err(|e| {
            StoreError::Custom(format!(
                "Failed to create RocksDB checkpoint at {path:?}: {e}"
            ))
        })?;

        Ok(())
    }

    fn flush(&self) -> Result<(), StoreError> {
        // Flush every column family's memtable to an SST file, then sync the WAL.
        // Together these make the next open a clean start: the memtables are
        // durable as SST and the WAL tail (anything still in the log) is fsynced,
        // so RocksDB does not have to replay the WAL on recovery.
        for table in TABLES {
            if let Some(cf) = self.db.cf_handle(table) {
                self.db.flush_cf(&cf).map_err(|e| {
                    StoreError::Custom(format!("RocksDB flush_cf({table}) failed: {e}"))
                })?;
            }
        }
        self.db
            .flush_wal(true)
            .map_err(|e| StoreError::Custom(format!("RocksDB flush_wal failed: {e}")))
    }

    fn stats(&self) -> Option<StorageStats> {
        Some(StorageStats {
            engine_statistics: self.statistics(),
            tables: self.table_stats(),
        })
    }
}

/// Read-only view for RocksDB
pub struct RocksDBReadTx {
    db: Arc<DBWithThreadMode<MultiThreaded>>,
}

impl StorageReadView for RocksDBReadTx {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

        self.db
            .get_cf(&cf, key)
            .map_err(|e| StoreError::Custom(format!("Failed to get from {}: {}", table, e)))
    }

    fn multi_get(
        &self,
        table: &'static str,
        keys: &[&[u8]],
    ) -> Vec<Result<Option<Vec<u8>>, StoreError>> {
        let Some(cf) = self.db.cf_handle(table) else {
            let err_msg = format!("Table {} not found", table);
            return (0..keys.len())
                .map(|_| Err(StoreError::Custom(err_msg.clone())))
                .collect();
        };
        // `sorted_input=false`: rocksdb sorts internally. Caller may pass arbitrary order.
        self.db
            .batched_multi_get_cf(&cf, keys.iter().copied(), false)
            .into_iter()
            .map(|res| {
                res.map(|opt| opt.map(|slice| slice.to_vec()))
                    .map_err(|e| StoreError::Custom(format!("multi_get {}: {}", table, e)))
            })
            .collect()
    }

    fn prefix_iterator(
        &self,
        table: &'static str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

        let iter = self.db.prefix_iterator_cf(&cf, prefix).map(|result| {
            result.map_err(|e| StoreError::Custom(format!("Failed to iterate: {e}")))
        });
        Ok(Box::new(iter))
    }

    fn first_key(&self, table: &'static str) -> Result<Option<Vec<u8>>, StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {table} not found")))?;
        let mut iter = self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start);
        match iter.next() {
            Some(Ok((k, _))) => Ok(Some(k.to_vec())),
            Some(Err(e)) => Err(StoreError::Custom(e.to_string())),
            None => Ok(None),
        }
    }

    fn last_key(&self, table: &'static str) -> Result<Option<Vec<u8>>, StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {table} not found")))?;
        let mut iter = self.db.iterator_cf(&cf, rocksdb::IteratorMode::End);
        match iter.next() {
            Some(Ok((k, _))) => Ok(Some(k.to_vec())),
            Some(Err(e)) => Err(StoreError::Custom(e.to_string())),
            None => Ok(None),
        }
    }
}

/// Write batch for RocksDB
pub struct RocksDBWriteTx {
    /// Database reference for writing
    db: Arc<DBWithThreadMode<MultiThreaded>>,
    /// Write batch for accumulating changes
    batch: WriteBatch,
}

impl StorageWriteBatch for RocksDBWriteTx {
    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {table:?} not found")))?;
        self.batch.put_cf(&cf, key, value);
        Ok(())
    }

    /// Stores multiple key-value pairs in a single table.
    /// Changes are accumulated in the batch and written atomically on commit.
    fn put_batch(
        &mut self,
        table: &'static str,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {table:?} not found")))?;

        for (key, value) in batch {
            self.batch.put_cf(&cf, key, value);
        }
        Ok(())
    }

    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

        self.batch.delete_cf(&cf, key);
        Ok(())
    }

    fn delete_range(
        &mut self,
        table: &'static str,
        start: &[u8],
        end: &[u8],
    ) -> Result<(), StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {table:?} not found")))?;
        self.batch.delete_range_cf(&cf, start, end);
        Ok(())
    }

    fn merge(&mut self, table: &'static str, key: &[u8], operand: &[u8]) -> Result<(), StoreError> {
        // Only TRANSACTION_LOCATIONS has a merge operator registered. Merging on
        // any other CF would enqueue an operand RocksDB can't resolve, deferring
        // the failure to read/compaction time where it's hard to diagnose — so
        // fail fast here instead.
        if table != TRANSACTION_LOCATIONS {
            return Err(StoreError::Custom(format!(
                "merge not supported for table {table} (no merge operator registered)"
            )));
        }
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

        self.batch.merge_cf(&cf, key, operand);
        Ok(())
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        // Take ownership of the batch (replaces it with an empty one) since db.write() consumes it
        let batch = std::mem::take(&mut self.batch);
        self.db
            .write(batch)
            .map_err(|e| StoreError::Custom(format!("Failed to commit batch: {}", e)))
    }
}

/// Locked snapshot for RocksDB
/// This is used for batch read operations in snap sync
pub struct RocksDBLocked {
    /// Reference to database
    db: &'static Arc<DBWithThreadMode<MultiThreaded>>,
    /// Snapshot/locked transaction
    lock: SnapshotWithThreadMode<'static, DBWithThreadMode<MultiThreaded>>,
    /// Column family handle
    cf: Arc<rocksdb::BoundColumnFamily<'static>>,
}

impl StorageLockedView for RocksDBLocked {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        self.lock
            .get_cf(&self.cf, key)
            .map_err(|e| StoreError::Custom(format!("Failed to get:{e:?}")))
    }
}

impl Drop for RocksDBLocked {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(
                self.db as *const Arc<DBWithThreadMode<MultiThreaded>>
                    as *mut Arc<DBWithThreadMode<MultiThreaded>>,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::encode_tx_location_operand;
    use ethrex_common::H256;
    use ethrex_common::types::{BlockHash, BlockNumber, Index};
    use ethrex_rlp::decode::RLPDecode;

    /// End-to-end guard for the associative merge operator at the real RocksDB
    /// layer: write many operands for the same key, each flushed into its own
    /// SST file, then force a compaction (which exercises the merge operator,
    /// including PartialMerge). Before the operand/value format fix this dropped
    /// entries during compaction (observed as 1664 silent drops on mainnet).
    #[test]
    fn merge_operator_survives_flush_and_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let backend = RocksDBBackend::open(dir.path(), RocksDBConfig::default()).unwrap();
        let cf = backend.db.cf_handle(TRANSACTION_LOCATIONS).unwrap();

        let tx_hash = H256::from_low_u64_be(0xabcd);
        let entries: Vec<(BlockNumber, BlockHash, Index)> = (0..6u64)
            .map(|i| (100 + i, H256::from_low_u64_be(0x10 + i), i))
            .collect();

        // Each operand in its own committed batch + flush → separate SST files.
        for (bn, bh, idx) in &entries {
            let mut tx = backend.begin_write().unwrap();
            tx.merge(
                TRANSACTION_LOCATIONS,
                tx_hash.as_bytes(),
                &encode_tx_location_operand(*bn, *bh, *idx),
            )
            .unwrap();
            tx.commit().unwrap();
            backend.db.flush_cf(&cf).unwrap();
        }

        // Force compaction — consolidates operands across the SST files.
        backend
            .db
            .compact_range_cf(&cf, None::<&[u8]>, None::<&[u8]>);

        let read = backend.begin_read().unwrap();
        let bytes = read
            .get(TRANSACTION_LOCATIONS, tx_hash.as_bytes())
            .unwrap()
            .expect("key must exist after merge + compaction");
        let mut got = <Vec<(BlockNumber, BlockHash, Index)>>::decode(&bytes).unwrap();
        got.sort();
        let mut want = entries;
        want.sort();
        assert_eq!(got, want, "no entries may be dropped through compaction");
    }

    /// Same-block_hash operands must dedupe to the latest, even across a
    /// flush+compaction boundary.
    #[test]
    fn merge_operator_dedupes_across_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let backend = RocksDBBackend::open(dir.path(), RocksDBConfig::default()).unwrap();
        let cf = backend.db.cf_handle(TRANSACTION_LOCATIONS).unwrap();

        let tx_hash = H256::from_low_u64_be(0x1234);
        let bh = H256::from_low_u64_be(0xaa);
        // Same block_hash written twice (e.g. re-import); later index must win.
        for idx in [3u64, 7u64] {
            let mut tx = backend.begin_write().unwrap();
            tx.merge(
                TRANSACTION_LOCATIONS,
                tx_hash.as_bytes(),
                &encode_tx_location_operand(200, bh, idx),
            )
            .unwrap();
            tx.commit().unwrap();
            backend.db.flush_cf(&cf).unwrap();
        }
        backend
            .db
            .compact_range_cf(&cf, None::<&[u8]>, None::<&[u8]>);

        let read = backend.begin_read().unwrap();
        let bytes = read
            .get(TRANSACTION_LOCATIONS, tx_hash.as_bytes())
            .unwrap()
            .unwrap();
        let got = <Vec<(BlockNumber, BlockHash, Index)>>::decode(&bytes).unwrap();
        assert_eq!(
            got,
            vec![(200, bh, 7)],
            "later write for same block_hash wins"
        );
    }

    /// The state tables carry `memtable_prefix_bloom_ratio(0.2)`, but a ratio
    /// on its own builds **nothing**: RocksDB only allocates a memtable bloom
    /// when a prefix extractor is set *or* whole-key filtering is on, and
    /// neither was. A devnet capture on 2026-08-08 confirmed it from the
    /// applied `OPTIONS` file — `prefix_extractor=nullptr`,
    /// `memtable_whole_key_filtering=false` on every trie and flat CF, MPT and
    /// binary alike. The ratio was dead configuration.
    ///
    /// This asserts the filter is *used*, not that a setter was called.
    /// `bloom_memtable_miss_count` is only incremented from inside
    /// `MemTable::Get`, and only when the memtable actually holds a bloom — so
    /// a non-zero count is RocksDB reporting it consulted a filter it built.
    /// Removing `set_memtable_whole_key_filtering(true)` drops it to 0.
    ///
    /// Deliberately never flushes: this must exercise the memtable path, not
    /// the SST bloom (which is a separate, already-working setting).
    #[test]
    fn the_memtable_bloom_is_real_on_the_state_tables() {
        use rocksdb::perf::{PerfContext, PerfMetric, PerfStatsLevel, set_perf_stats};

        for table in [
            ACCOUNT_TRIE_NODES,
            STORAGE_TRIE_NODES,
            BINARY_TRIE_NODES,
            ACCOUNT_FLATKEYVALUE,
            STORAGE_FLATKEYVALUE,
            BINARY_FLATKEYVALUE,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let backend = RocksDBBackend::open(dir.path(), RocksDBConfig::default()).unwrap();

            // Populate the memtable only — no flush, so every read below is
            // served (or rejected) by the memtable.
            let mut tx = backend.begin_write().unwrap();
            for i in 0..4096u64 {
                tx.put(table, H256::from_low_u64_be(i).as_bytes(), b"node")
                    .unwrap();
            }
            tx.commit().unwrap();

            let read = backend.begin_read().unwrap();

            set_perf_stats(PerfStatsLevel::EnableCount);
            let mut ctx = PerfContext::default();
            ctx.reset();
            // Keys that were never written: the bloom should reject them
            // without walking the skiplist.
            for i in 0..1024u64 {
                let absent = H256::from_low_u64_be(u64::MAX - i);
                assert!(read.get(table, absent.as_bytes()).unwrap().is_none());
            }
            let misses = ctx.metric(PerfMetric::BloomMemtableMissCount);
            ctx.reset();
            // Keys that were written: the bloom should pass them through.
            for i in 0..1024u64 {
                let present = H256::from_low_u64_be(i);
                assert!(read.get(table, present.as_bytes()).unwrap().is_some());
            }
            let hits = ctx.metric(PerfMetric::BloomMemtableHitCount);
            set_perf_stats(PerfStatsLevel::Disable);

            assert!(
                misses > 0,
                "{table}: 1024 absent point lookups produced no memtable-bloom \
                 rejections, so no memtable bloom was built. \
                 memtable_prefix_bloom_ratio alone does not create one — it \
                 needs memtable_whole_key_filtering or a prefix extractor."
            );
            assert!(
                hits > 0,
                "{table}: present keys did not register memtable-bloom hits"
            );
        }
    }

    /// The applied `OPTIONS` file is RocksDB's own record of what it actually
    /// runs with, and is what the devnet had to read by hand. Asserting on it
    /// catches the case where a setter exists, compiles, and is silently
    /// ignored or overridden by a later call.
    #[test]
    fn the_applied_options_file_records_whole_key_filtering() {
        let dir = tempfile::tempdir().unwrap();
        let _backend = RocksDBBackend::open(dir.path(), RocksDBConfig::default()).unwrap();

        // RocksDB writes OPTIONS-<n> on open; take the highest-numbered one.
        let options_file = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("OPTIONS-"))
            .max()
            .expect("RocksDB must write an OPTIONS file on open");
        let text = std::fs::read_to_string(dir.path().join(options_file)).unwrap();

        // Split into `[CFOptions "name"]` sections.
        let section_of = |cf: &str| -> String {
            let header = format!("[CFOptions \"{cf}\"]");
            let start = text
                .find(&header)
                .unwrap_or_else(|| panic!("no {header} section in OPTIONS"))
                + header.len();
            let rest = &text[start..];
            let end = rest.find("\n[").unwrap_or(rest.len());
            rest[..end].to_owned()
        };

        for table in [
            ACCOUNT_TRIE_NODES,
            STORAGE_TRIE_NODES,
            BINARY_TRIE_NODES,
            ACCOUNT_FLATKEYVALUE,
            STORAGE_FLATKEYVALUE,
            BINARY_FLATKEYVALUE,
        ] {
            let section = section_of(table);
            assert!(
                section.contains("memtable_whole_key_filtering=true"),
                "{table}: OPTIONS does not enable whole-key memtable filtering, \
                 so memtable_prefix_bloom_size_ratio is a no-op:\n{section}"
            );
            assert!(
                section.contains("memtable_prefix_bloom_size_ratio=0.200000"),
                "{table}: OPTIONS does not size the memtable bloom:\n{section}"
            );
        }
    }

    /// Pulls a ticker's count out of RocksDB's statistics dump. Lines look
    /// like `rocksdb.bloom.filter.useful COUNT : 1234`.
    fn ticker(dump: &str, name: &str) -> u64 {
        dump.lines()
            .find(|line| line.starts_with(name))
            .and_then(|line| line.rsplit(':').next())
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or_else(|| panic!("ticker {name} missing from dump:\n{dump}"))
    }

    /// Statistics cost throughput (RocksDB documents ~5-10%), so the default
    /// must not pay for them. Asserts absence of the *object*, not of a config
    /// bit: with no `Statistics` installed RocksDB has nothing to dump.
    #[test]
    fn statistics_are_absent_unless_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let backend = RocksDBBackend::open(dir.path(), RocksDBConfig::default()).unwrap();
        assert!(!RocksDBConfig::default().enable_statistics);

        let stats = backend.stats().expect("rocksdb always reports table stats");
        assert!(
            stats.engine_statistics.is_none(),
            "a default node must not be collecting RocksDB statistics"
        );
        assert!(
            !stats.tables.is_empty(),
            "per-table properties are free and must be readable regardless"
        );
    }

    /// The finding this closes: with no `Statistics` object anywhere in the
    /// tree, the bloom tickers were not recorded, so a devnet could not tell
    /// whether the SST bloom was doing anything.
    ///
    /// Asserts the *count*, not the presence of the ticker name — the name
    /// appears in every dump, including one where every counter is zero.
    #[test]
    fn enabling_statistics_records_the_bloom_tickers() {
        let dir = tempfile::tempdir().unwrap();
        let backend = RocksDBBackend::open(
            dir.path(),
            RocksDBConfig {
                enable_statistics: true,
                ..RocksDBConfig::default()
            },
        )
        .unwrap();

        let mut tx = backend.begin_write().unwrap();
        for i in 0..4096u64 {
            tx.put(BINARY_TRIE_NODES, H256::from_low_u64_be(i).as_bytes(), b"n")
                .unwrap();
        }
        tx.commit().unwrap();
        // Flush so the reads below hit an SST and consult its bloom filter;
        // an unflushed memtable would exercise the memtable bloom instead.
        let cf = backend.db.cf_handle(BINARY_TRIE_NODES).unwrap();
        backend.db.flush_cf(&cf).unwrap();

        let read = backend.begin_read().unwrap();
        for i in 0..1024u64 {
            let absent = H256::from_low_u64_be(u64::MAX - i);
            assert!(
                read.get(BINARY_TRIE_NODES, absent.as_bytes())
                    .unwrap()
                    .is_none()
            );
        }

        let dump = backend
            .stats()
            .unwrap()
            .engine_statistics
            .expect("statistics were enabled at open");
        assert!(
            ticker(&dump, "rocksdb.bloom.filter.useful") > 0,
            "absent-key reads against an SST must show the bloom avoiding file \
             reads; this is the number the 2026-08-08 devnet could not obtain"
        );
        assert!(
            ticker(&dump, "rocksdb.number.keys.written") >= 4096,
            "write tickers must be recorded too"
        );
    }

    /// Per-table sizes and key counts must be readable without the statistics
    /// object, and must actually describe the table asked about — the devnet
    /// had to stop the node and count SST files by hand for these.
    #[test]
    fn table_stats_describe_each_table_separately() {
        let dir = tempfile::tempdir().unwrap();
        let backend = RocksDBBackend::open(dir.path(), RocksDBConfig::default()).unwrap();

        const KEYS: u64 = 2048;
        let mut tx = backend.begin_write().unwrap();
        for i in 0..KEYS {
            tx.put(BINARY_TRIE_NODES, H256::from_low_u64_be(i).as_bytes(), b"n")
                .unwrap();
        }
        tx.commit().unwrap();

        let of = |stats: &StorageStats, table: &str| -> TableStats {
            stats
                .tables
                .iter()
                .find(|t| t.table == table)
                .expect("every table is reported")
                .clone()
        };

        // Before any flush everything lives in the memtable. This is exactly
        // the state the 2026-08-08 devnet was in (zero flushes), where the SST
        // figures describe nothing.
        let before = backend.stats().unwrap();
        let unflushed = of(&before, BINARY_TRIE_NODES);
        assert!(
            unflushed.memtable_bytes > 0,
            "unflushed writes must show up as memtable bytes"
        );
        assert_eq!(
            unflushed.sst_bytes, 0,
            "nothing has been flushed, so there is no SST"
        );

        let cf = backend.db.cf_handle(BINARY_TRIE_NODES).unwrap();
        backend.db.flush_cf(&cf).unwrap();

        let after = backend.stats().unwrap();
        let flushed = of(&after, BINARY_TRIE_NODES);
        assert_eq!(
            flushed.estimated_keys, KEYS,
            "key count must be per-table and exact for a freshly flushed table"
        );
        assert!(flushed.sst_bytes > 0, "the flush must have produced an SST");
        assert!(flushed.live_data_bytes > 0, "live data must be non-zero");

        // A table nobody wrote to must not inherit the numbers above.
        let untouched = of(&after, ACCOUNT_TRIE_NODES);
        assert_eq!(untouched.estimated_keys, 0);
        assert_eq!(untouched.sst_bytes, 0);
        assert_eq!(untouched.live_data_bytes, 0);
    }

    /// Every table must have a *deliberate* column-family home — either a
    /// tuning arm, or this list saying the catch-all default is intended.
    ///
    /// Silence is the failure mode. `BINARY_TRIE_NODES` and
    /// `BINARY_FLATKEYVALUE` were added with no arm and silently took the
    /// default: 64MB buffers and **no bloom filter**, while their MPT
    /// counterparts get 512MB and 10 bits per key. Nothing failed — the tables
    /// worked, they were just slow in a way no test could see.
    ///
    /// These are recorded as intended, not measured. A devnet run on 2026-08-08
    /// could not measure column-family behaviour at all: the datadir was ~5MB
    /// against a 12GiB shared cache with zero flushes, so no SST existed and no
    /// bloom ever bound. Any of these may deserve tuning once there is a state
    /// large enough to show it.
    const DELIBERATELY_DEFAULT: &[(&str, &str)] = &[
        (
            "ACCOUNT_CODE_METADATA",
            "small, paired with the tuned ACCOUNT_CODES",
        ),
        ("BAD_BLOCKS", "diagnostic, rarely written and rarely read"),
        (
            "BINARY_TRIE_ROOTS",
            "one small row per block, read once at import",
        ),
        (
            "BLOCK_ACCESS_LISTS",
            "per block, but read only during validation",
        ),
        (
            "CHAIN_DATA",
            "a handful of rows for the lifetime of the datadir",
        ),
        (
            "EXECUTION_WITNESSES",
            "large values, written and read whole",
        ),
        (
            "FULLSYNC_HEADERS",
            "transient, cleared when full sync completes",
        ),
        ("INVALID_CHAINS", "diagnostic"),
        (
            "MISC_VALUES",
            "a handful of rows; holds the generator frontiers",
        ),
        (
            "PENDING_BLOCKS",
            "transient, small, drained as blocks are canonicalised",
        ),
        ("SNAP_STATE", "transient, cleared when snap sync completes"),
        (
            "STATE_HISTORY",
            "write-per-block and range-deleted at finality; a bloom does not \
             help a range scan, and the reorg read is rare",
        ),
    ];

    /// Fails when a table is added to `TABLES` without a column-family
    /// decision, which is how the binary tables ended up unblooomed.
    ///
    /// Scans the source because RocksDB's applied options are not
    /// introspectable from the handle, and counts rather than name-matches
    /// because `TABLES` holds string values while the arms name constants.
    #[test]
    fn every_table_has_a_deliberate_column_family_home() {
        use crate::api::tables::TABLES;

        let source = include_str!("rocksdb.rs");
        let tuned: std::collections::BTreeSet<String> = source
            .lines()
            .filter(|line| line.trim_end().ends_with("=> {"))
            .flat_map(|line| {
                line.split(|c: char| !(c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()))
                    .filter(|t| t.len() > 3)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect();

        let defaulted: std::collections::BTreeSet<String> = DELIBERATELY_DEFAULT
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .collect();

        let overlap: Vec<_> = tuned.intersection(&defaulted).collect();
        assert!(
            overlap.is_empty(),
            "these tables are both tuned and listed as deliberately default, so \
             one of the two is stale: {overlap:?}"
        );

        assert_eq!(
            tuned.len() + defaulted.len(),
            TABLES.len(),
            "every table needs a home: {} tuned + {} deliberately default != {} \
             in TABLES. A table added without an arm silently takes the \
             catch-all -- no bloom filter, a 64MB write buffer -- which is what \
             happened to the binary-trie tables. Add an arm, or add it to \
             DELIBERATELY_DEFAULT with the reason.",
            tuned.len(),
            defaulted.len(),
            TABLES.len()
        );

        // The hot state tables are not a judgement call: they are read on the
        // descent and must carry the MPT's tuning.
        for table in [
            "ACCOUNT_TRIE_NODES",
            "STORAGE_TRIE_NODES",
            "ACCOUNT_FLATKEYVALUE",
            "STORAGE_FLATKEYVALUE",
            "BINARY_TRIE_NODES",
            "BINARY_FLATKEYVALUE",
        ] {
            assert!(tuned.contains(table), "{table} must be explicitly tuned");
        }
    }
}
