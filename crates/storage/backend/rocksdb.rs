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
    BlockBasedOptions, ColumnFamilyDescriptor, MultiThreaded, Options, SnapshotWithThreadMode,
    WriteBatch,
};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

/// RocksDB backend
#[derive(Debug)]
pub struct RocksDBBackend {
    /// Optimistric transaction database
    db: Arc<DBWithThreadMode<MultiThreaded>>,
}

impl RocksDBBackend {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
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

        let compressible_tables = [
            BLOCK_NUMBERS,
            HEADERS,
            BODIES,
            RECEIPTS,
            TRANSACTION_LOCATIONS,
            FULLSYNC_HEADERS,
        ];

        // opts.enable_statistics();
        // opts.set_stats_dump_period_sec(600);

        // Open all column families
        let existing_cfs = DBWithThreadMode::<MultiThreaded>::list_cf(&opts, path.as_ref())
            .unwrap_or_else(|_| vec!["default".to_string()]);

        let mut all_cfs_to_open = HashSet::new();
        all_cfs_to_open.extend(existing_cfs.iter().cloned());
        all_cfs_to_open.extend(TABLES.iter().map(|table| table.to_string()));

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
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                CANONICAL_BLOCK_HASHES | BLOCK_NUMBERS => {
                    cf_opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(128 * 1024 * 1024); // 128MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024); // 16KB
                    block_opts.set_bloom_filter(10.0, false);
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
                    block_opts.set_bloom_filter(10.0, false); // 10 bits per key
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
                    block_opts.set_bloom_filter(10.0, false); // 10 bits per key
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
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                RECEIPTS => {
                    cf_opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(32 * 1024); // 32KB
                    cf_opts.set_block_based_table_factory(&block_opts);
                }
                _ => {
                    // Default for other CFs
                    cf_opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
                    cf_opts.set_max_write_buffer_number(3);
                    cf_opts.set_target_file_size_base(128 * 1024 * 1024); // 128MB

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024);
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

        // Clean up obsolete column families
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
        Ok(Self { db: Arc::new(db) })
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

    fn begin_read(&self) -> Result<Box<dyn StorageReadView + '_>, StoreError> {
        Ok(Box::new(RocksDBReadTx {
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

    fn set_sync_mode(&self) -> Result<(), StoreError> {
        let core_count = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);

        // Relax compaction triggers to avoid write stalls during bulk ingestion.
        // During sync we write faster than compaction can keep up, so we allow
        // more L0 files to accumulate before triggering slowdowns/stops.
        let sync_cf_opts: &[(&str, &str)] = &[
            ("level0_file_num_compaction_trigger", "64"),
            ("level0_slowdown_writes_trigger", "128"),
            ("level0_stop_writes_trigger", "256"),
        ];

        let write_buffer_count = core_count.to_string();
        let trie_sync_opts: Vec<(&str, &str)> = {
            let mut opts = sync_cf_opts.to_vec();
            opts.push(("max_write_buffer_number", &write_buffer_count));
            opts
        };

        let trie_cfs = [
            ACCOUNT_TRIE_NODES,
            STORAGE_TRIE_NODES,
            ACCOUNT_FLATKEYVALUE,
            STORAGE_FLATKEYVALUE,
        ];

        for cf_name in trie_cfs {
            if let Some(cf) = self.db.cf_handle(cf_name) {
                self.db
                    .set_options_cf(&cf, &trie_sync_opts)
                    .map_err(|e| {
                        StoreError::Custom(format!(
                            "Failed to set sync options for {cf_name}: {e}"
                        ))
                    })?;
            }
        }

        // Relax default DB-level compaction triggers too
        self.db.set_options(sync_cf_opts).map_err(|e| {
            StoreError::Custom(format!("Failed to set DB-level sync options: {e}"))
        })?;

        info!(
            "RocksDB sync mode enabled: relaxed compaction triggers, {} write buffers for trie CFs",
            core_count
        );
        Ok(())
    }

    fn set_normal_mode(&self) -> Result<(), StoreError> {
        // Restore normal compaction triggers (matching values from open())
        let trie_normal_opts: &[(&str, &str)] = &[
            ("level0_file_num_compaction_trigger", "4"),
            ("level0_slowdown_writes_trigger", "20"),
            ("level0_stop_writes_trigger", "36"),
            ("max_write_buffer_number", "6"),
        ];

        let default_normal_opts: &[(&str, &str)] = &[
            ("level0_file_num_compaction_trigger", "4"),
            ("level0_slowdown_writes_trigger", "20"),
            ("level0_stop_writes_trigger", "36"),
        ];

        let trie_cfs = [
            ACCOUNT_TRIE_NODES,
            STORAGE_TRIE_NODES,
            ACCOUNT_FLATKEYVALUE,
            STORAGE_FLATKEYVALUE,
        ];

        for cf_name in trie_cfs {
            if let Some(cf) = self.db.cf_handle(cf_name) {
                self.db
                    .set_options_cf(&cf, trie_normal_opts)
                    .map_err(|e| {
                        StoreError::Custom(format!(
                            "Failed to restore normal options for {cf_name}: {e}"
                        ))
                    })?;
            }
        }

        self.db.set_options(default_normal_opts).map_err(|e| {
            StoreError::Custom(format!("Failed to restore DB-level normal options: {e}"))
        })?;

        info!("RocksDB normal mode restored, starting manual compaction on trie CFs");

        // Trigger manual compaction on trie CFs to consolidate L0 files
        // accumulated during sync.
        for cf_name in trie_cfs {
            if let Some(cf) = self.db.cf_handle(cf_name) {
                self.db
                    .compact_range_cf(&cf, None::<&[u8]>, None::<&[u8]>);
                info!("Triggered compaction for {cf_name}");
            }
        }

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
