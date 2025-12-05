use crate::api::{
    PrefixResult, StorageBackend, StorageLockedView, StorageReadView, StorageWriteBatch,
    tables::TABLES,
};
use crate::error::StoreError;
use libmdbx::{Database, DatabaseOptions, TableFlags, Transaction, WriteFlags, WriteMap};
use std::path::Path;
use std::sync::Arc;

/// Libmdbx backend
#[derive(Debug)]
pub struct LibmdbxBackend {
    db: Arc<Database<WriteMap>>,
}

impl LibmdbxBackend {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        // Rocksdb optimizations options
        let opts = DatabaseOptions::default();

        let db = Database::<WriteMap>::open_with_options(path, opts).unwrap();

        let tx = db.begin_rw_txn().unwrap();

        for table_name in TABLES {
            let flags = TableFlags::empty();
            tx.create_table(Some(&table_name), flags).unwrap();
        }

        tx.commit().unwrap();

        Ok(Self { db: Arc::new(db) })
    }
}

impl StorageBackend for LibmdbxBackend {
    fn clear_table(&self, table: &'static str) -> Result<(), StoreError> {
        let tx = self.db.begin_rw_txn().unwrap();
        let table_handle = tx.open_table(Some(table)).unwrap();
        tx.clear_table(&table_handle).unwrap();
        tx.commit().unwrap();
        Ok(())
    }

    fn begin_read(&self) -> Result<Box<dyn StorageReadView + '_>, StoreError> {
        Ok(Box::new(LibmdbxReadTx {
            db: self.db.clone(),
        }))
    }

    fn begin_write(&self) -> Result<Box<dyn StorageWriteBatch + 'static>, StoreError> {
        let db = Box::leak(Box::new(self.db.clone()));
        let tx = db.begin_rw_txn().unwrap();

        Ok(Box::new(LibmdbxWriteTx { db, tx }))
    }

    fn begin_locked(
        &self,
        table_name: &'static str,
    ) -> Result<Box<dyn StorageLockedView>, StoreError> {
        let db = Box::leak(Box::new(self.db.clone()));
        let tx = db.begin_ro_txn().unwrap();

        Ok(Box::new(LibmdbxLocked {
            db,
            tx,
            table: table_name,
        }))
    }

    fn create_checkpoint(&self, path: &Path) -> Result<(), StoreError> {
        todo!()
    }
}

/// Read-only view for Libmdbx
pub struct LibmdbxReadTx {
    db: Arc<Database<WriteMap>>,
}

impl StorageReadView for LibmdbxReadTx {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let tx = self.db.begin_ro_txn().unwrap();
        let table_handle = tx.open_table(Some(table)).unwrap();
        tx.get(&table_handle, key)
            .map_err(|e| StoreError::Custom(format!("Failed to get from {}: {}", table, e)))
    }

    fn prefix_iterator(
        &self,
        table: &'static str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError> {
        todo!()
        // let tx = self.db.begin_ro_txn().unwrap();
        // let table_handle = tx.open_table(Some(table)).unwrap();
        // let iter = tx
        //     .cursor(&table_handle)
        //     .unwrap()
        //     .into_iter_from(prefix)
        //     .map(|result: Result<(Vec<u8>, Vec<u8>), _>| {
        //         result
        //             .map(|(k, v)| (k.into_boxed_slice(), v.into_boxed_slice()))
        //             .map_err(|e| StoreError::Custom(format!("Failed to iterate: {e}")))
        //     })
        //     .take_while(|result| {
        //         result
        //             .as_ref()
        //             .map(|(k, _)| k.starts_with(prefix))
        //             .unwrap_or(true)
        //     });
        // Ok(Box::new(iter))
    }
}

/// Write batch for Libmdbx
pub struct LibmdbxWriteTx {
    db: &'static Arc<Database<WriteMap>>,
    tx: Transaction<'static, libmdbx::RW, WriteMap>,
}

impl StorageWriteBatch for LibmdbxWriteTx {
    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        let table_handle = self.tx.open_table(Some(table)).unwrap();
        self.tx
            .put(&table_handle, key, value, WriteFlags::empty())
            .unwrap();
        Ok(())
    }

    /// Stores multiple key-value pairs in a single table.
    /// Changes are accumulated in the batch and written atomically on commit.
    fn put_batch(
        &mut self,
        table: &'static str,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        let table_handle = self.tx.open_table(Some(table)).unwrap();

        for (key, value) in batch {
            self.tx
                .put(&table_handle, key, value, WriteFlags::empty())
                .unwrap();
        }
        Ok(())
    }

    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        let table_handle = self.tx.open_table(Some(table)).unwrap();

        self.tx.del(&table_handle, key, None).unwrap();
        Ok(())
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        let old_tx = std::mem::replace(&mut self.tx, self.db.begin_rw_txn().unwrap());
        old_tx.commit().unwrap();
        Ok(())
    }
}

impl Drop for LibmdbxWriteTx {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(
                self.db as *const Arc<Database<WriteMap>> as *mut Arc<Database<WriteMap>>,
            ));
        }
    }
}

/// Locked snapshot for Libmdbx
/// This is used for batch read operations in snap sync
pub struct LibmdbxLocked {
    db: &'static Arc<Database<WriteMap>>,
    tx: Transaction<'static, libmdbx::RO, WriteMap>,
    table: &'static str,
}

impl StorageLockedView for LibmdbxLocked {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let table_handle = self.tx.open_table(Some(self.table)).unwrap();
        self.tx
            .get(&table_handle, key)
            .map_err(|e| StoreError::Custom(format!("Failed to get:{e:?}")))
    }
}

impl Drop for LibmdbxLocked {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(
                self.db as *const Arc<Database<WriteMap>> as *mut Arc<Database<WriteMap>>,
            ));
        }
    }
}
