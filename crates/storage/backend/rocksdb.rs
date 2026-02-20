use crate::api::tables::{
    ACCOUNT_CODES, ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, BLOCK_NUMBERS, BODIES,
    CANONICAL_BLOCK_HASHES, FULLSYNC_HEADERS, HEADERS, RECEIPTS, STORAGE_FLATKEYVALUE,
    STORAGE_TRIE_NODES, TRANSACTION_LOCATIONS,
};
use crate::api::{
    PrefixResult, StorageBackend, StorageLockedView, StorageReadView, StorageWriteBatch,
    tables::TABLES,
};
use crate::error::StoreError;
use rocksdb::DBWithThreadMode;
use rocksdb::checkpoint::Checkpoint;
use rocksdb::{
    BlockBasedOptions, Cache, ColumnFamilyDescriptor, MultiThreaded, Options,
    SnapshotWithThreadMode, WriteBatch,
};
use std::collections::HashSet;
use std::env;
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Debug, Clone)]
struct RocksDbPerfTuning {
    block_cache_bytes: Option<usize>,
    cache_index_and_filter_blocks: bool,
    pin_l0_filter_and_index_blocks: bool,
    optimize_filters_for_hits: bool,
    cache_trie_only: bool,
    disable_state_bloom_filter: bool,
    advise_random_on_open: Option<bool>,
    use_direct_reads: Option<bool>,
    read_only_max_open_files: Option<i32>,
    read_only_file_opening_threads: Option<i32>,
    enable_statistics: bool,
    stats_dump_period_sec: Option<u32>,
}

impl RocksDbPerfTuning {
    fn from_env() -> Self {
        let block_cache_mb = env::var("ETHREX_PERF_ROCKSDB_BLOCK_CACHE_MB")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0);
        let block_cache_bytes = block_cache_mb
            .map(|mb| mb.saturating_mul(1024).saturating_mul(1024))
            .filter(|bytes| *bytes > 0);

        let stats_dump_period_sec = env::var("ETHREX_PERF_ROCKSDB_STATS_DUMP_SEC")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v > 0);
        let read_only_max_open_files = env::var("ETHREX_PERF_ROCKSDB_READ_ONLY_MAX_OPEN_FILES")
            .ok()
            .and_then(|v| v.parse::<i32>().ok());
        let read_only_file_opening_threads =
            env::var("ETHREX_PERF_ROCKSDB_READ_ONLY_FILE_OPENING_THREADS")
                .ok()
                .and_then(|v| v.parse::<i32>().ok())
                .filter(|v| *v > 0);

        Self {
            block_cache_bytes,
            cache_index_and_filter_blocks: parse_bool_env(
                "ETHREX_PERF_ROCKSDB_CACHE_INDEX_FILTER",
                false,
            ),
            pin_l0_filter_and_index_blocks: parse_bool_env(
                "ETHREX_PERF_ROCKSDB_PIN_L0_INDEX_FILTER",
                false,
            ),
            optimize_filters_for_hits: parse_bool_env(
                "ETHREX_PERF_ROCKSDB_OPTIMIZE_FILTERS_FOR_HITS",
                false,
            ),
            cache_trie_only: parse_bool_env("ETHREX_PERF_ROCKSDB_CACHE_TRIE_ONLY", false),
            disable_state_bloom_filter: parse_bool_env(
                "ETHREX_PERF_ROCKSDB_DISABLE_STATE_BLOOM_FILTER",
                false,
            ),
            advise_random_on_open: parse_optional_bool_env(
                "ETHREX_PERF_ROCKSDB_ADVISE_RANDOM_ON_OPEN",
            ),
            use_direct_reads: parse_optional_bool_env("ETHREX_PERF_ROCKSDB_USE_DIRECT_READS"),
            read_only_max_open_files,
            read_only_file_opening_threads,
            enable_statistics: parse_bool_env("ETHREX_PERF_ROCKSDB_ENABLE_STATS", false),
            stats_dump_period_sec,
        }
    }

    fn enabled(&self) -> bool {
        self.block_cache_bytes.is_some()
            || self.cache_index_and_filter_blocks
            || self.pin_l0_filter_and_index_blocks
            || self.optimize_filters_for_hits
            || self.cache_trie_only
            || self.disable_state_bloom_filter
            || self.advise_random_on_open.is_some()
            || self.use_direct_reads.is_some()
            || self.read_only_max_open_files.is_some()
            || self.read_only_file_opening_threads.is_some()
            || self.enable_statistics
            || self.stats_dump_period_sec.is_some()
    }
}

fn parse_bool_env(name: &str, default: bool) -> bool {
    match env::var(name) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        },
        Err(_) => default,
    }
}

fn parse_optional_bool_env(name: &str) -> Option<bool> {
    match env::var(name) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        },
        Err(_) => None,
    }
}

fn apply_perf_block_options(
    block_opts: &mut BlockBasedOptions,
    tuning: &RocksDbPerfTuning,
    shared_block_cache: Option<&Cache>,
    apply_block_cache: bool,
) {
    if apply_block_cache && let Some(cache) = shared_block_cache {
        block_opts.set_block_cache(cache);
    }
    if tuning.cache_index_and_filter_blocks {
        block_opts.set_cache_index_and_filter_blocks(true);
    }
    if tuning.pin_l0_filter_and_index_blocks {
        block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
    }
}

/// RocksDB backend
#[derive(Debug)]
pub struct RocksDBBackend {
    /// Optimistric transaction database
    db: Arc<DBWithThreadMode<MultiThreaded>>,
    /// Whether the DB was opened in read-only mode.
    read_only: bool,
}

impl RocksDBBackend {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_internal(path, false)
    }

    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_internal(path, true)
    }

    fn open_internal(path: impl AsRef<Path>, read_only: bool) -> Result<Self, StoreError> {
        // Rocksdb optimizations options
        let mut opts = Options::default();
        let perf_tuning = RocksDbPerfTuning::from_env();
        opts.create_if_missing(!read_only);
        opts.create_missing_column_families(!read_only);

        if read_only {
            // Replay/planning tools open a second handle to an already-running
            // live DB. Keep FD usage bounded to avoid EMFILE on production hosts.
            opts.set_max_open_files(perf_tuning.read_only_max_open_files.unwrap_or(256));
            opts.set_max_file_opening_threads(
                perf_tuning.read_only_file_opening_threads.unwrap_or(4),
            );
        } else {
            opts.set_max_open_files(-1);
            opts.set_max_file_opening_threads(16);
        }

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
        opts.set_advise_random_on_open(perf_tuning.advise_random_on_open.unwrap_or(false));
        if let Some(use_direct_reads) = perf_tuning.use_direct_reads {
            opts.set_use_direct_reads(use_direct_reads);
        }
        opts.set_compression_type(rocksdb::DBCompressionType::None);

        if perf_tuning.optimize_filters_for_hits {
            opts.set_optimize_filters_for_hits(true);
        }
        if perf_tuning.enable_statistics {
            opts.enable_statistics();
            if let Some(period) = perf_tuning.stats_dump_period_sec {
                opts.set_stats_dump_period_sec(period);
            }
        }

        let compressible_tables = [
            BLOCK_NUMBERS,
            HEADERS,
            BODIES,
            RECEIPTS,
            TRANSACTION_LOCATIONS,
            FULLSYNC_HEADERS,
        ];
        let shared_block_cache = perf_tuning.block_cache_bytes.map(Cache::new_lru_cache);
        if perf_tuning.enabled() {
            info!(
                read_only,
                ?perf_tuning,
                "RocksDB performance tuning enabled via environment"
            );
        }

        // Open all column families
        let existing_cfs = DBWithThreadMode::<MultiThreaded>::list_cf(&opts, path.as_ref())
            .unwrap_or_else(|_| vec!["default".to_string()]);

        let mut all_cfs_to_open = HashSet::new();
        all_cfs_to_open.extend(existing_cfs.iter().cloned());
        if !read_only {
            all_cfs_to_open.extend(TABLES.iter().map(|table| table.to_string()));
        }

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
                    apply_perf_block_options(
                        &mut block_opts,
                        &perf_tuning,
                        shared_block_cache.as_ref(),
                        true,
                    );
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                CANONICAL_BLOCK_HASHES | BLOCK_NUMBERS => {
                    cf_opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(128 * 1024 * 1024); // 128MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024); // 16KB
                    block_opts.set_bloom_filter(10.0, false);
                    apply_perf_block_options(
                        &mut block_opts,
                        &perf_tuning,
                        shared_block_cache.as_ref(),
                        !perf_tuning.cache_trie_only,
                    );
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                ACCOUNT_TRIE_NODES | STORAGE_TRIE_NODES => {
                    cf_opts.set_write_buffer_size(512 * 1024 * 1024); // 512MB
                    cf_opts.set_max_write_buffer_number(6);
                    cf_opts.set_min_write_buffer_number_to_merge(2);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB
                    cf_opts.set_memtable_prefix_bloom_ratio(0.2); // Bloom filter

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024); // 16KB
                    if !perf_tuning.disable_state_bloom_filter {
                        block_opts.set_bloom_filter(10.0, false); // 10 bits per key
                    }
                    apply_perf_block_options(
                        &mut block_opts,
                        &perf_tuning,
                        shared_block_cache.as_ref(),
                        true,
                    );
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                ACCOUNT_FLATKEYVALUE | STORAGE_FLATKEYVALUE => {
                    cf_opts.set_write_buffer_size(512 * 1024 * 1024); // 512MB
                    cf_opts.set_max_write_buffer_number(6);
                    cf_opts.set_min_write_buffer_number_to_merge(2);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB
                    cf_opts.set_memtable_prefix_bloom_ratio(0.2); // Bloom filter

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024); // 16KB
                    if !perf_tuning.disable_state_bloom_filter {
                        block_opts.set_bloom_filter(10.0, false); // 10 bits per key
                    }
                    apply_perf_block_options(
                        &mut block_opts,
                        &perf_tuning,
                        shared_block_cache.as_ref(),
                        true,
                    );
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
                    apply_perf_block_options(
                        &mut block_opts,
                        &perf_tuning,
                        shared_block_cache.as_ref(),
                        !perf_tuning.cache_trie_only,
                    );
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                RECEIPTS => {
                    cf_opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(32 * 1024); // 32KB
                    apply_perf_block_options(
                        &mut block_opts,
                        &perf_tuning,
                        shared_block_cache.as_ref(),
                        !perf_tuning.cache_trie_only,
                    );
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                _ => {
                    // Default for other CFs
                    cf_opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(128 * 1024 * 1024); // 128MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024);
                    apply_perf_block_options(
                        &mut block_opts,
                        &perf_tuning,
                        shared_block_cache.as_ref(),
                        !perf_tuning.cache_trie_only,
                    );
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
            }

            cf_descriptors.push(ColumnFamilyDescriptor::new(cf_name, cf_opts));
        }

        let db = if read_only {
            DBWithThreadMode::<MultiThreaded>::open_cf_descriptors_read_only(
                &opts,
                path.as_ref(),
                cf_descriptors,
                false,
            )
            .map_err(|e| StoreError::Custom(format!("Failed to open RocksDB (read-only): {}", e)))?
        } else {
            DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
                &opts,
                path.as_ref(),
                cf_descriptors,
            )
            .map_err(|e| {
                StoreError::Custom(format!("Failed to open RocksDB with all CFs: {}", e))
            })?
        };

        // Clean up obsolete column families
        if !read_only {
            for cf_name in &existing_cfs {
                if cf_name != "default" && !TABLES.contains(&cf_name.as_str()) {
                    warn!("Dropping obsolete column family: {}", cf_name);
                    let _ = db
                        .drop_cf(cf_name)
                        .inspect(|_| info!("Successfully dropped column family: {}", cf_name))
                        .inspect_err(|e|
                            // Log error but don't fail initialization - the database is still usable
                            warn!("Failed to drop column family '{}': {}", cf_name, e));
                }
            }
        }
        Ok(Self {
            db: Arc::new(db),
            read_only,
        })
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
        if self.read_only {
            return Err(StoreError::Custom(
                "cannot clear table on read-only RocksDB backend".to_string(),
            ));
        }

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
        if self.read_only {
            return Err(StoreError::Custom(
                "cannot open write transaction on read-only RocksDB backend".to_string(),
            ));
        }

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
