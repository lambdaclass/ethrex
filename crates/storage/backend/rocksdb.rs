use crate::api::tables::{
    ACCOUNT_CODES, ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, BLOCK_NUMBERS, BODIES,
    CANONICAL_BLOCK_HASHES, FULLSYNC_HEADERS, HEADERS, RECEIPTS_V2, STORAGE_FLATKEYVALUE,
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
    BlockBasedOptions, Cache, ColumnFamilyDescriptor, MergeOperands, MultiThreaded, Options,
    SnapshotWithThreadMode, WriteBatch,
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

/// RocksDB backend
#[derive(Debug)]
pub struct RocksDBBackend {
    /// Optimistric transaction database
    db: Arc<DBWithThreadMode<MultiThreaded>>,
}

impl RocksDBBackend {
    pub fn open(path: impl AsRef<Path>, block_cache_size: usize) -> Result<Self, StoreError> {
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
                ACCOUNT_TRIE_NODES | STORAGE_TRIE_NODES => {
                    cf_opts.set_write_buffer_size(512 * 1024 * 1024); // 512MB
                    cf_opts.set_max_write_buffer_number(6);
                    cf_opts.set_min_write_buffer_number_to_merge(2);
                    cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB
                    cf_opts.set_memtable_prefix_bloom_ratio(0.2); // Bloom filter

                    let mut block_opts = BlockBasedOptions::default();
                    block_opts.set_block_size(16 * 1024); // 16KB
                    block_opts.set_bloom_filter(10.0, false); // 10 bits per key
                    configure_block_cache(&mut block_opts);
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

        Ok(Self { db: Arc::new(db) })
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
        from: &[u8],
        to: &[u8],
    ) -> Result<(), StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {table:?} not found")))?;
        // Half-open [from, to); accumulated in the batch so it commits atomically
        // with the batch's other mutations (e.g. the healed node write).
        self.batch.delete_range_cf(&cf, from, to);
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

    /// `delete_range` on the real backend removes the half-open `[from, to)`
    /// interval per CF, and multiple CFs range-deleted in one batch commit
    /// atomically (the whole-account-gone fan-out, E13).
    #[test]
    fn delete_range_spans_cfs_in_one_batch() {
        use crate::api::tables::{ACCOUNT_TRIE_NODES, STORAGE_TRIE_NODES};
        let dir = tempfile::tempdir().unwrap();
        let backend = RocksDBBackend::open(
            dir.path(),
            crate::store::DEFAULT_ROCKSDB_BLOCK_CACHE_SIZE_BYTES,
        )
        .unwrap();

        let mut w = backend.begin_write().unwrap();
        for k in 0u8..=5 {
            w.put(ACCOUNT_TRIE_NODES, &[k], b"a").unwrap();
            w.put(STORAGE_TRIE_NODES, &[k], b"s").unwrap();
        }
        w.commit().unwrap();

        // Same range deleted on two CFs in one atomic batch.
        let mut w = backend.begin_write().unwrap();
        w.delete_range(ACCOUNT_TRIE_NODES, &[2], &[4]).unwrap();
        w.delete_range(STORAGE_TRIE_NODES, &[2], &[4]).unwrap();
        w.commit().unwrap();

        let r = backend.begin_read().unwrap();
        for k in 0u8..=5 {
            let expected = k != 2 && k != 3;
            assert_eq!(
                r.get(ACCOUNT_TRIE_NODES, &[k]).unwrap().is_some(),
                expected,
                "account_trie_nodes key {k}"
            );
            assert_eq!(
                r.get(STORAGE_TRIE_NODES, &[k]).unwrap().is_some(),
                expected,
                "storage_trie_nodes key {k}"
            );
        }
    }

    /// End-to-end guard for the associative merge operator at the real RocksDB
    /// layer: write many operands for the same key, each flushed into its own
    /// SST file, then force a compaction (which exercises the merge operator,
    /// including PartialMerge). Before the operand/value format fix this dropped
    /// entries during compaction (observed as 1664 silent drops on mainnet).
    #[test]
    fn merge_operator_survives_flush_and_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let backend = RocksDBBackend::open(
            dir.path(),
            crate::store::DEFAULT_ROCKSDB_BLOCK_CACHE_SIZE_BYTES,
        )
        .unwrap();
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
        let backend = RocksDBBackend::open(
            dir.path(),
            crate::store::DEFAULT_ROCKSDB_BLOCK_CACHE_SIZE_BYTES,
        )
        .unwrap();
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
}
