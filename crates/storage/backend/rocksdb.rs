use crate::api::{
    PrefixResult, StorageBackend, StorageLocked, StorageRoTx, StorageRwTx, TABLES, TableOptions,
};
use crate::error::StoreError;
use rocksdb::{
    ColumnFamilyDescriptor, MultiThreaded, Options, SnapshotWithThreadMode, Transaction,
};
use rocksdb::{OptimisticTransactionDB, WriteBatchWithTransaction};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

#[derive(Debug)]
pub struct RocksDBBackend {
    db: Arc<OptimisticTransactionDB<MultiThreaded>>,
}

impl StorageBackend for RocksDBBackend {
    fn open(path: impl AsRef<Path>) -> Result<Self, StoreError>
    where
        Self: Sized,
    {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        // Basic optimizations
        opts.set_max_open_files(-1);
        opts.set_max_background_jobs(8);

        // Write optimizations
        opts.set_write_buffer_size(128 * 1024 * 1024); // 128MB
        opts.set_max_write_buffer_number(4);
        opts.set_min_write_buffer_number_to_merge(2);

        // WAL optimizations
        opts.set_use_fsync(false); // Use fdatasync instead of fsync
        opts.set_enable_pipelined_write(true);
        opts.set_allow_concurrent_memtable_write(true);

        let existing_cfs = OptimisticTransactionDB::<MultiThreaded>::list_cf(&opts, path.as_ref())
            .unwrap_or_else(|_| vec!["default".to_string()]);

        let mut all_cfs_to_open = HashSet::new();
        all_cfs_to_open.extend(existing_cfs.iter().cloned());
        all_cfs_to_open.extend(TABLES.iter().map(|table| table.to_string()));

        let cf_descriptors = all_cfs_to_open
            .iter()
            .map(|cf| ColumnFamilyDescriptor::new(cf, Options::default()))
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
        // Create a self-contained snapshot that can live independently
        // We use Box::leak because RocksDB's Snapshot needs 'static references to the DB
        // This is necessary for long-lived trie operations in snap sync
        let db = Box::leak(Box::new(self.db.clone()));
        let lock = db.snapshot();
        let cf = db
            .cf_handle(table_name)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table_name)))?;

        Ok(Box::new(RocksDBLocked { db, lock, cf }))
    }
}

pub struct RocksDBRoTx<'a> {
    tx: Transaction<'a, OptimisticTransactionDB<MultiThreaded>>,
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

pub struct RocksDBPrefixIter {
    results: std::vec::IntoIter<PrefixResult>,
}

impl Iterator for RocksDBPrefixIter {
    type Item = Result<(Vec<u8>, Vec<u8>), StoreError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.results.next()
    }
}

pub struct RocksDBRwTx<'a> {
    db: Arc<OptimisticTransactionDB<MultiThreaded>>,
    tx: Transaction<'a, OptimisticTransactionDB<MultiThreaded>>,
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
