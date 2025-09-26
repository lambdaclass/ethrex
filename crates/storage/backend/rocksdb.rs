use crate::api::{
    PrefixResult, StorageBackend, StorageLocked, StorageRoTx, StorageRwTx, TABLES, TableOptions,
};
use crate::error::StoreError;
use rocksdb::{
    BlockBasedOptions, Cache, ColumnFamilyDescriptor, DBCompressionType, MultiThreaded, Options,
    SliceTransform, SnapshotWithThreadMode, Transaction,
};
use rocksdb::{OptimisticTransactionDB, WriteBatchWithTransaction};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

/// RocksDB backend
#[derive(Debug)]
pub struct RocksDBBackend {
    /// Optimistric transaction database
    db: Arc<OptimisticTransactionDB<MultiThreaded>>,
}

impl StorageBackend for RocksDBBackend {
    fn open(path: impl AsRef<Path>) -> Result<Self, StoreError>
    where
        Self: Sized,
    {
        // RocksDB options (DB-level)
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        opts.set_max_open_files(-1);
        opts.set_max_background_jobs(8);
        opts.set_level_compaction_dynamic_level_bytes(true);
        opts.set_enable_pipelined_write(true);
        opts.set_allow_concurrent_memtable_write(true);
        opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
        opts.set_max_write_buffer_number(4);
        opts.set_min_write_buffer_number_to_merge(2);
        opts.set_bytes_per_sync(32 * 1024 * 1024); // 32MB

        // Shared block cache
        let cache = Cache::new_lru_cache(512 * 1024 * 1024); // 512MB

        // Open all column families
        let existing_cfs = OptimisticTransactionDB::<MultiThreaded>::list_cf(&opts, path.as_ref())
            .unwrap_or_else(|_| vec!["default".to_string()]);

        let mut all_cfs_to_open = HashSet::new();
        all_cfs_to_open.extend(existing_cfs.iter().cloned());
        all_cfs_to_open.extend(TABLES.iter().map(|table| table.to_string()));

        // Per-CF tuning
        let cf_descriptors = all_cfs_to_open
            .iter()
            .map(|cf_name| {
                let mut cf_opts = Options::default();

                // Defaults for any CF
                cf_opts.set_level_compaction_dynamic_level_bytes(true);
                cf_opts.set_compression_type(DBCompressionType::Lz4);
                cf_opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
                cf_opts.set_max_write_buffer_number(3);
                cf_opts.set_target_file_size_base(128 * 1024 * 1024); // 128MB

                // Block-based table options with shared cache
                let mut bb = BlockBasedOptions::default();
                bb.set_block_cache(&cache);
                bb.set_block_size(16 * 1024); // 16KB por defecto
                bb.set_cache_index_and_filter_blocks(true);

                match cf_name.as_str() {
                    "headers" | "bodies" => {
                        cf_opts.set_compression_type(DBCompressionType::Zstd);
                        cf_opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
                        cf_opts.set_max_write_buffer_number(4);
                        cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB

                        bb.set_block_size(32 * 1024); // 32KB
                    }
                    "canonical_block_hashes" | "block_numbers" => {
                        cf_opts.set_write_buffer_size(64 * 1024 * 1024);
                        cf_opts.set_max_write_buffer_number(3);
                        cf_opts.set_target_file_size_base(128 * 1024 * 1024);
                        bb.set_bloom_filter(10.0, false);
                    }
                    "state_trie_nodes" | "storage_trie_nodes" => {
                        cf_opts.set_write_buffer_size(256 * 1024 * 1024); // 256MB
                        cf_opts.set_max_write_buffer_number(6);
                        cf_opts.set_min_write_buffer_number_to_merge(2);
                        cf_opts.set_target_file_size_base(256 * 1024 * 1024); // 256MB
                        cf_opts.set_memtable_prefix_bloom_ratio(0.2);

                        bb.set_bloom_filter(10.0, false);
                        bb.set_pin_l0_filter_and_index_blocks_in_cache(true);

                        if cf_name == "storage_trie_nodes" {
                            cf_opts.set_prefix_extractor(SliceTransform::create_fixed_prefix(32));
                        }
                    }
                    "transaction_locations" => {
                        cf_opts.set_write_buffer_size(64 * 1024 * 1024);
                        cf_opts.set_max_write_buffer_number(3);
                        cf_opts.set_target_file_size_base(128 * 1024 * 1024);
                        bb.set_bloom_filter(10.0, false);
                        cf_opts.set_prefix_extractor(SliceTransform::create_fixed_prefix(32));
                    }
                    "receipts" | "account_codes" => {
                        cf_opts.set_write_buffer_size(128 * 1024 * 1024);
                        cf_opts.set_max_write_buffer_number(3);
                        cf_opts.set_target_file_size_base(256 * 1024 * 1024);
                        bb.set_block_size(32 * 1024);
                    }
                    _ => {
                        // Default for other CFs
                    }
                }

                cf_opts.set_block_based_table_factory(&bb);

                ColumnFamilyDescriptor::new(cf_name, cf_opts)
            })
            .collect::<Vec<_>>();

        let db = OptimisticTransactionDB::<MultiThreaded>::open_cf_descriptors(
            &opts,
            path.as_ref(),
            cf_descriptors,
        )
        .map_err(|e| StoreError::Custom(format!("Failed to open RocksDB with all CFs: {}", e)))?;
        Ok(Self { db: Arc::new(db) })
    }

    fn create_table(&self, _name: &str, _options: TableOptions) -> Result<(), StoreError> {
        // Now we are creating the tables in the open() function
        // Check if this function is still needed
        Ok(())
    }

    fn clear_table(&self, table: &str) -> Result<(), StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom("Column family not found".to_string()))?;

        let mut iter = self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start);
        let mut batch = WriteBatchWithTransaction::<true>::default();

        while let Some(Ok((key, _))) = iter.next() {
            batch.delete_cf(&cf, key);
        }

        self.db
            .write(batch)
            .map_err(|e| StoreError::Custom(format!("RocksDB batch write error: {}", e)))
    }

    fn begin_read(&self) -> Result<Box<dyn StorageRoTx + '_>, StoreError> {
        let tx = self.db.transaction();

        let mut cfs: HashMap<String, Arc<rocksdb::BoundColumnFamily<'_>>> =
            HashMap::with_capacity(TABLES.len());
        for &table in TABLES.iter() {
            let cf = self
                .db
                .cf_handle(table)
                .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;
            cfs.insert(table.to_string(), cf);
        }
        Ok(Box::new(RocksDBRoTx { tx, cfs }))
    }

    fn begin_write(&self) -> Result<Box<dyn StorageRwTx + '_>, StoreError> {
        let tx = self.db.transaction();

        let mut cfs: HashMap<String, Arc<rocksdb::BoundColumnFamily<'_>>> =
            HashMap::with_capacity(TABLES.len());
        for &table in TABLES.iter() {
            let cf = self
                .db
                .cf_handle(table)
                .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;
            cfs.insert(table.to_string(), cf);
        }
        Ok(Box::new(RocksDBRwTx {
            db: self.db.clone(),
            tx,
            cfs,
        }))
    }

    fn begin_locked(&self, table_name: &str) -> Result<Box<dyn StorageLocked>, StoreError> {
        let db = Box::leak(Box::new(self.db.clone()));
        let lock = db.snapshot();
        let cf = db
            .cf_handle(table_name)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table_name)))?;

        Ok(Box::new(RocksDBLocked { db, lock, cf }))
    }
}

/// Read-only transaction for RocksDB
pub struct RocksDBRoTx<'a> {
    /// Transaction
    tx: Transaction<'a, OptimisticTransactionDB<MultiThreaded>>,
    /// Hashmap of column families
    cfs: HashMap<String, Arc<rocksdb::BoundColumnFamily<'a>>>,
}

impl<'a> StorageRoTx for RocksDBRoTx<'a> {
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let cf = self
            .cfs
            .get(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?
            .clone();

        self.tx
            .get_cf(&cf, key)
            .map_err(|e| StoreError::Custom(format!("Failed to get from {}: {}", table, e)))
    }

    fn prefix_iterator(
        &self,
        table: &str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<u8>), StoreError>> + '_>, StoreError>
    {
        let cf = self
            .cfs
            .get(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?
            .clone();

        let iter = self.tx.prefix_iterator_cf(&cf, prefix);
        let results: Vec<PrefixResult> = iter
            .map(|result| {
                result
                    .map(|(k, v)| (k.to_vec(), v.to_vec()))
                    .map_err(|e| StoreError::Custom(format!("Failed to iterate: {e}")))
            })
            .collect();

        let rocks_iter = RocksDBPrefixIter {
            results: results.into_iter(),
        };
        Ok(Box::new(rocks_iter))
    }
}

/// Prefix iterator for RocksDB
pub struct RocksDBPrefixIter {
    /// Vector of prefix results
    results: std::vec::IntoIter<PrefixResult>,
}

impl Iterator for RocksDBPrefixIter {
    type Item = Result<(Vec<u8>, Vec<u8>), StoreError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.results.next()
    }
}

/// Read-write transaction for RocksDB
pub struct RocksDBRwTx<'a> {
    /// Reference to database
    db: Arc<OptimisticTransactionDB<MultiThreaded>>,
    /// Transaction
    tx: Transaction<'a, OptimisticTransactionDB<MultiThreaded>>,
    /// Hashmap of column families
    cfs: HashMap<String, Arc<rocksdb::BoundColumnFamily<'a>>>,
}

impl<'a> StorageRoTx for RocksDBRwTx<'a> {
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let cf = self
            .cfs
            .get(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?
            .clone();

        self.tx
            .get_cf(&cf, key)
            .map_err(|e| StoreError::Custom(format!("Failed to get from {}: {}", table, e)))
    }

    fn prefix_iterator(
        &self,
        table: &str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<u8>), StoreError>> + '_>, StoreError>
    {
        let cf = self
            .cfs
            .get(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?
            .clone();

        let iter = self.tx.prefix_iterator_cf(&cf, prefix);
        let results: Vec<PrefixResult> = iter
            .map(|result| {
                result
                    .map(|(k, v)| (k.to_vec(), v.to_vec()))
                    .map_err(|e| StoreError::Custom(format!("Failed to iterate: {e}")))
            })
            .collect();

        let rocks_iter = RocksDBPrefixIter {
            results: results.into_iter(),
        };
        Ok(Box::new(rocks_iter))
    }
}

impl<'a> StorageRwTx for RocksDBRwTx<'a> {
    /// Stores multiple key-value pairs in different tables using [`WriteBatchWithTransaction`].
    /// This struct needs [`OptimisticTransactionDB`] to write the changes to the database.
    /// This method doesn't need to [`commit()`](StorageRwTx::commit) the transaction because it is done internally.
    fn put_batch(&self, batch: Vec<(&str, Vec<u8>, Vec<u8>)>) -> Result<(), StoreError> {
        let mut write_batch = WriteBatchWithTransaction::<true>::default();
        for (table, key, value) in batch {
            let cf = self
                .cfs
                .get(table)
                .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?
                .clone();

            write_batch.put_cf(&cf, key.as_slice(), value.as_slice());
        }

        self.db
            .write(write_batch)
            .map_err(|e| StoreError::Custom(format!("Failed to write batch: {}", e)))
    }

    fn delete(&self, table: &str, key: &[u8]) -> Result<(), StoreError> {
        let cf = self
            .cfs
            .get(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?
            .clone();

        self.tx
            .delete_cf(&cf, key)
            .map_err(|e| StoreError::Custom(format!("Failed to delete from {}: {}", table, e)))
    }

    fn commit(self: Box<Self>) -> Result<(), StoreError> {
        self.tx
            .commit()
            .map_err(|e| StoreError::Custom(format!("Failed to commit transaction: {}", e)))
    }
}

/// Locked snapshot for RocksDB
/// This is used for batch read operations in snap sync
pub struct RocksDBLocked {
    /// Reference to database
    db: &'static Arc<OptimisticTransactionDB<MultiThreaded>>,
    /// Snapshot/locked transaction
    lock: SnapshotWithThreadMode<'static, OptimisticTransactionDB<MultiThreaded>>,
    /// Column family handle
    cf: Arc<rocksdb::BoundColumnFamily<'static>>,
}

impl StorageLocked for RocksDBLocked {
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
                self.db as *const Arc<OptimisticTransactionDB<MultiThreaded>>
                    as *mut Arc<OptimisticTransactionDB<MultiThreaded>>,
            ));
        }
    }
}
