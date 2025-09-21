use crate::api::{StorageBackend, StorageLocked, StorageRoTx, StorageRwTx, TABLES, TableOptions};
use crate::error::StoreError;
use rocksdb::ColumnFamilyDescriptor;
use rocksdb::{
    MultiThreaded, OptimisticTransactionDB, Options, SnapshotWithThreadMode, Transaction,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub struct RocksDBBackend {
    db: Arc<OptimisticTransactionDB<MultiThreaded>>,
}

impl RocksDBBackend {}

impl StorageBackend for RocksDBBackend {
    fn open(path: impl AsRef<Path>) -> Result<Arc<Self>, StoreError>
    where
        Self: Sized,
    {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        // Abrimos/creamos todos los CFs conocidos al inicio para evitar lookups fallidos
        let cf_descriptors: Vec<_> = TABLES
            .iter()
            .map(|&name| ColumnFamilyDescriptor::new(name, Options::default()))
            .collect();

        let db = OptimisticTransactionDB::<MultiThreaded>::open_cf_descriptors(
            &opts,
            path.as_ref(),
            cf_descriptors,
        )
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

    fn begin_locked(&self, table_name: &str) -> Result<Box<dyn StorageLocked + '_>, StoreError> {
        let lock = self.db.snapshot();
        let cf = self
            .db
            .cf_handle(table_name)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table_name)))?
            .clone();

        Ok(Box::new(RocksDBLocked { lock, cf }))
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
        let results: Vec<Result<(Vec<u8>, Vec<u8>), StoreError>> = iter
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
    cfs: HashMap<String, Arc<rocksdb::BoundColumnFamily<'a>>>,
}

// Primero implementamos StorageRoTx para RocksDBRwTx
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
        let results: Vec<Result<(Vec<u8>, Vec<u8>), StoreError>> = iter
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

// Ahora implementamos StorageRwTx
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

pub struct RocksDBLocked<'a> {
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
