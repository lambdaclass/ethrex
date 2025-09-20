use crate::api::{StorageBackend, StorageLocked, StorageRoTx, StorageRwTx, TableOptions};
use crate::error::StoreError;
use rocksdb::{
    MultiThreaded, OptimisticTransactionDB, Options, SnapshotWithThreadMode, Transaction,
};
use std::path::Path;
use std::sync::Arc;

#[derive(Debug)]
pub struct RocksDBBackend {
    db: Arc<OptimisticTransactionDB<MultiThreaded>>,
}

impl StorageBackend for RocksDBBackend {
    type ReadTx<'a> = RocksDBRoTx<'a>;
    type WriteTx<'a> = RocksDBRwTx<'a>;
    type Locked<'a> = RocksDBLocked<'a>;

    fn open(path: impl AsRef<Path>) -> Result<Arc<Self>, StoreError>
    where
        Self: Sized,
    {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let db = OptimisticTransactionDB::open(&opts, path)
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

    fn begin_read(&self) -> Result<Self::ReadTx<'_>, StoreError> {
        let tx = self.db.transaction();
        Ok(RocksDBRoTx { tx, db: &self.db })
    }

    fn begin_write(&self) -> Result<Self::WriteTx<'_>, StoreError> {
        let tx = self.db.transaction();
        Ok(RocksDBRwTx { tx, db: &self.db })
    }

    fn begin_locked(&self, table_name: &str) -> Result<Self::Locked<'_>, StoreError> {
        let lock = self.db.snapshot();
        let cf = self
            .db
            .cf_handle(table_name)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table_name)))?
            .clone();

        Ok(RocksDBLocked {
            db: self.db.clone(),
            lock,
            cf,
        })
    }
}

pub struct RocksDBRoTx<'a> {
    tx: Transaction<'a, OptimisticTransactionDB<MultiThreaded>>,
    db: &'a OptimisticTransactionDB<MultiThreaded>,
}

impl<'a> StorageRoTx for RocksDBRoTx<'a> {
    type PrefixIter = RocksDBPrefixIter;
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

        self.tx
            .get_cf(&cf, key)
            .map_err(|e| StoreError::Custom(format!("Failed to get from {}: {}", table, e)))
    }

    fn prefix_iterator(&self, table: &str, prefix: &[u8]) -> Result<Self::PrefixIter, StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?
            .clone();

        let iter = self.tx.prefix_iterator_cf(&cf, prefix);
        let results: Vec<Result<(Vec<u8>, Vec<u8>), StoreError>> = iter
            .map(|result| {
                result
                    .map(|(k, v)| (k.to_vec(), v.to_vec()))
                    .map_err(|e| StoreError::Custom(format!("Failed to iterate: {e}")))
            })
            .collect();

        Ok(RocksDBPrefixIter {
            results: results.into_iter(),
        })
    }
}

pub struct RocksDBPrefixIter {
    results: std::vec::IntoIter<Result<(Vec<u8>, Vec<u8>), StoreError>>,
}

impl Iterator for RocksDBPrefixIter {
    type Item = Result<(Vec<u8>, Vec<u8>), StoreError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.results.next()
    }
}

pub struct RocksDBRwTx<'a> {
    tx: Transaction<'a, OptimisticTransactionDB<MultiThreaded>>,
    db: &'a OptimisticTransactionDB<MultiThreaded>,
}

// Primero implementamos StorageRoTx para RocksDBRwTx
impl<'a> StorageRoTx for RocksDBRwTx<'a> {
    type PrefixIter = RocksDBPrefixIter;

    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

        self.tx
            .get_cf(&cf, key)
            .map_err(|e| StoreError::Custom(format!("Failed to get from {}: {}", table, e)))
    }

    fn prefix_iterator(&self, table: &str, prefix: &[u8]) -> Result<Self::PrefixIter, StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?
            .clone();

        let iter = self.tx.prefix_iterator_cf(&cf, prefix);
        let results: Vec<Result<(Vec<u8>, Vec<u8>), StoreError>> = iter
            .map(|result| {
                result
                    .map(|(k, v)| (k.to_vec(), v.to_vec()))
                    .map_err(|e| StoreError::Custom(format!("Failed to iterate: {e}")))
            })
            .collect();

        Ok(RocksDBPrefixIter {
            results: results.into_iter(),
        })
    }
}

// Ahora implementamos StorageRwTx
impl<'a> StorageRwTx for RocksDBRwTx<'a> {
    fn put(&self, table: &str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

        self.tx
            .put_cf(&cf, key, value)
            .map_err(|e| StoreError::Custom(format!("Failed to put to {}: {}", table, e)))
    }

    fn delete(&self, table: &str, key: &[u8]) -> Result<(), StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

        self.tx
            .delete_cf(&cf, key)
            .map_err(|e| StoreError::Custom(format!("Failed to delete from {}: {}", table, e)))
    }

    fn commit(self) -> Result<(), StoreError> {
        self.tx
            .commit()
            .map_err(|e| StoreError::Custom(format!("Failed to commit transaction: {}", e)))
    }
}

pub struct RocksDBLocked<'a> {
    /// Database reference para mantener el snapshot vivo
    db: Arc<OptimisticTransactionDB<MultiThreaded>>,
    /// Snapshot/locked transaction
    lock: SnapshotWithThreadMode<'a, OptimisticTransactionDB<MultiThreaded>>,
    /// Column family handle
    cf: Arc<rocksdb::BoundColumnFamily<'a>>,
}

impl<'a> StorageLocked for RocksDBLocked<'a> {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        self.lock
            .get_cf(&self.cf, key)
            .map_err(|e| StoreError::Custom(format!("Failed to get:{e:?}")))
    }
}
