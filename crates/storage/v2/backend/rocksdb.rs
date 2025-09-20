use crate::error::StoreError;
use crate::v2::api::{StorageBackend, StorageRoTx, StorageRwTx, TableOptions};
use rocksdb::{MultiThreaded, OptimisticTransactionDB, Options, Transaction};
use std::sync::Arc;

#[derive(Debug)]
pub struct RocksDBBackend {
    db: Arc<OptimisticTransactionDB<MultiThreaded>>,
}

impl StorageBackend for RocksDBBackend {
    fn open(path: &str) -> Result<Arc<dyn StorageBackend>, StoreError>
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

    fn begin_read<'a>(&'a self) -> Result<Box<dyn StorageRoTx<'a> + 'a>, StoreError> {
        let tx = self.db.transaction();
        Ok(Box::new(RocksDBRoTx { tx, db: &self.db }))
    }

    fn begin_write<'a>(&'a self) -> Result<Box<dyn StorageRwTx<'a> + 'a>, StoreError> {
        let tx = self.db.transaction();
        Ok(Box::new(RocksDBRwTx { tx, db: &self.db }))
    }
}

pub struct RocksDBRoTx<'a> {
    tx: Transaction<'a, OptimisticTransactionDB<MultiThreaded>>,
    db: &'a OptimisticTransactionDB<MultiThreaded>,
}

impl StorageRoTx<'_> for RocksDBRoTx<'_> {
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let cf = self
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

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
            .db
            .cf_handle(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

        let iter = self.tx.prefix_iterator_cf(&cf, prefix);
        let mapped_iter = iter.map(|result| {
            result
                .map(|(k, v)| (k.to_vec(), v.to_vec()))
                .map_err(|e| StoreError::Custom(format!("Failed to iterate: {e}")))
        });

        Ok(Box::new(mapped_iter))
    }
}

pub struct RocksDBRwTx<'a> {
    tx: Transaction<'a, OptimisticTransactionDB<MultiThreaded>>,
    db: &'a OptimisticTransactionDB<MultiThreaded>,
}

impl StorageRwTx<'_> for RocksDBRwTx<'_> {
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

    fn commit(self: Box<Self>) -> Result<(), StoreError> {
        self.tx
            .commit()
            .map_err(|e| StoreError::Custom(format!("Failed to commit transaction: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rocksdb_backend() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let backend = RocksDBBackend::open(temp_dir.path().to_str().unwrap()).unwrap();
        backend
            .create_table("test", TableOptions { dupsort: false })
            .unwrap();
        let tx = backend.begin_read().unwrap();
        let value = tx.get("test", b"test").unwrap();
        assert_eq!(value, None);
    }

    #[test]
    fn test_rocksdb_backend_write() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let backend = RocksDBBackend::open(temp_dir.path().to_str().unwrap()).unwrap();
        backend
            .create_table("test", TableOptions { dupsort: false })
            .unwrap();
        let txn = backend.begin_write().unwrap();
        txn.put("test", b"test", b"test").unwrap();
        txn.commit().unwrap();
        let txn = backend.begin_read().unwrap();
        let value = txn.get("test", b"test").unwrap();
        assert_eq!(value, Some(b"test".to_vec()));

        let value = txn.get("test", b"test2").unwrap();
        assert_eq!(value, None);
    }
}
