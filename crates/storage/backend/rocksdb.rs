use crate::api::{
    PrefixResult, StorageBackend, StorageLocked, StorageRoTx, StorageRwTx, TABLES, TableOptions,
};
use crate::error::StoreError;
use rocksdb::{
    BlockBasedOptions, ColumnFamilyDescriptor, MultiThreaded, Options, SnapshotWithThreadMode,
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
        // Rocksdb optimizations options
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        opts.set_max_open_files(-1);
        opts.set_max_file_opening_threads(16);

        opts.set_use_fsync(false); // fdatasync

        opts.enable_statistics();
        opts.set_stats_dump_period_sec(600);

        // Values taken from https://github.com/facebook/rocksdb/wiki/Setup-Options-and-Basic-Tuning#other-general-options
        // These are 'real' default values
        opts.set_level_compaction_dynamic_level_bytes(true);
        opts.set_max_background_jobs(6);
        opts.set_bytes_per_sync(1048576);
        opts.set_compaction_pri(rocksdb::CompactionPri::MinOverlappingRatio);

        // Open all column families
        let existing_cfs = OptimisticTransactionDB::<MultiThreaded>::list_cf(&opts, path.as_ref())
            .unwrap_or_else(|_| vec!["default".to_string()]);

        let mut all_cfs_to_open = HashSet::new();
        all_cfs_to_open.extend(existing_cfs.iter().cloned());
        all_cfs_to_open.extend(TABLES.iter().map(|table| table.to_string()));

        let cf_descriptors = all_cfs_to_open
            .iter()
            .map(|cf| {
                let mut cf_opts = Options::default();
                let mut bb_opts = BlockBasedOptions::default();
                bb_opts.set_block_size(16 * 1024);
                bb_opts.set_cache_index_and_filter_blocks(true);
                bb_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
                // Newest in this [list](https://github.com/facebook/rocksdb/blob/v8.6.7/include/rocksdb/table.h#L493-L521)
                // We can check main
                bb_opts.set_format_version(6);
                cf_opts.set_block_based_table_factory(&bb_opts);
                ColumnFamilyDescriptor::new(cf, cf_opts)
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

    fn create_table(&self, _name: &'static str, _options: TableOptions) -> Result<(), StoreError> {
        // Now we are creating the tables in the open() function
        // Check if this function is still needed
        Ok(())
    }

    fn clear_table(&self, table: &'static str) -> Result<(), StoreError> {
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
        Ok(Box::new(RocksDBRoTx {
            db: self.db.clone(),
        }))
    }

    fn begin_write(&self) -> Result<Box<dyn StorageRwTx + 'static>, StoreError> {
        let batch = WriteBatchWithTransaction::<true>::default();

        Ok(Box::new(RocksDBRwTx {
            db: self.db.clone(),
            batch,
        }))
    }

    fn begin_locked(&self, table_name: &'static str) -> Result<Box<dyn StorageLocked>, StoreError> {
        let db = Box::leak(Box::new(self.db.clone()));
        let lock = db.snapshot();
        let cf = db
            .cf_handle(table_name)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table_name)))?;

        Ok(Box::new(RocksDBLocked { db, lock, cf }))
    }
}

/// Read-only transaction for RocksDB
pub struct RocksDBRoTx {
    /// Transaction
    db: Arc<OptimisticTransactionDB<MultiThreaded>>,
}

impl StorageRoTx for RocksDBRoTx {
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

/// Read-write transaction for RocksDB
pub struct RocksDBRwTx {
    /// Database reference for writing
    db: Arc<OptimisticTransactionDB<MultiThreaded>>,
    /// Write batch for accumulating changes
    batch: WriteBatchWithTransaction<true>,
}

impl StorageRoTx for RocksDBRwTx {
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

impl StorageRwTx for RocksDBRwTx {
    /// Stores multiple key-value pairs in different tables using WriteBatch.
    /// Changes are accumulated in the batch and written atomically on commit.
    fn put_batch(
        &mut self,
        batch: Vec<(&'static str, Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        // Fast-path if we have only one table in the batch
        let Some(first_table) = batch.first().map(|(t, _, _)| *t) else {
            // Empty batch
            return Ok(());
        };

        if batch.iter().all(|(t, _, _)| *t == first_table) {
            // All tables are the same
            let cf = self
                .db
                .cf_handle(first_table)
                .ok_or_else(|| StoreError::Custom(format!("Table {} not found", first_table)))?;

            for (_, key, value) in batch {
                self.batch.put_cf(&cf, key, value);
            }
            return Ok(());
        }

        // Load the column families for the tables in the batch
        let mut cfs = HashMap::new();
        for (table, _, _) in &batch {
            if !cfs.contains_key(table) {
                let cf = self
                    .db
                    .cf_handle(table)
                    .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;
                cfs.insert(*table, cf);
            }
        }

        for (table, key, value) in batch {
            let cf = cfs
                .get(table)
                .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;
            self.batch.put_cf(cf, key, value);
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
