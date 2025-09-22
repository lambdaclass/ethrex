use crate::api::{
    PrefixResult, StorageBackend, StorageLocked, StorageRoTx, StorageRwTx, TABLES, TableOptions,
};
use crate::error::StoreError;
use rocksdb::{MultiThreaded, Options, SnapshotWithThreadMode, Transaction};
use rocksdb::{OptimisticTransactionDB, WriteBatchWithTransaction};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug)]
pub struct RocksDBBackend {
    db: Arc<OptimisticTransactionDB<MultiThreaded>>,
}

impl StorageBackend for RocksDBBackend {
    fn open(path: impl AsRef<Path>) -> Result<Arc<Self>, StoreError>
    where
        Self: Sized,
    {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let db = OptimisticTransactionDB::<MultiThreaded>::open(&opts, path.as_ref())
            .map_err(|e| StoreError::Custom(format!("Failed to open RocksDB: {}", e)))?;

        Ok(Arc::new(Self { db: Arc::new(db) }))
    }

    fn create_table(&self, name: &str, _options: TableOptions) -> Result<(), StoreError> {
        let opts = Options::default();
        self.db
            .create_cf(name, &opts)
            .map_err(|e| StoreError::Custom(format!("Failed to create table {}: {}", name, e)))
    }

    fn clear_table(&self, table: &str) -> Result<(), StoreError> {
        self.db
            .drop_cf(table)
            .map_err(|e| StoreError::Custom(format!("Failed to clear table {}: {}", table, e)))
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
        Ok(Box::new(RocksDBRwTx { tx, cfs }))
    }

    fn begin_locked(&self, table_name: &str) -> Result<Box<dyn StorageLocked>, StoreError> {
        let db = Box::leak(Box::new(self.db.clone()));
        let lock = db.snapshot();
        let cf = db
            .cf_handle(table_name)
            .ok_or_else(|| StoreError::Custom(format!("Tabla {} no encontrada", table_name)))?;

        Ok(Box::new(RocksDBLocked { db, lock, cf }))
    }

    fn write_batch(&self, table: &str, batch: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

        let mut write_batch = WriteBatchWithTransaction::<true>::default();
        for (key, value) in batch {
            write_batch.put_cf(&cf, key, value);
        }

        self.db
            .write(write_batch)
            .map_err(|e| StoreError::Custom(format!("Failed to write batch: {}", e)))
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
    fn put(&self, table: &str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        let cf = self
            .cfs
            .get(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?
            .clone();

        self.tx
            .put_cf(&cf, key, value)
            .map_err(|e| StoreError::Custom(format!("Failed to put to {}: {}", table, e)))
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
