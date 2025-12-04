//! CanopyDB storage backend implementation.
//! We use a single Environment with one database per table.
//! Each database has a single tree for key-value storage.
use canopydb::{
    DbOptions, EnvOptions, Environment, ReadTransaction, TreeOptions, WriteTransaction,
};

use crate::api::{
    PrefixResult, StorageBackend, StorageLocked, StorageReadTx, StorageWriteTx, tables::TABLES,
};
use crate::error::StoreError;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// CanopyDB backend
#[derive(Debug, Clone)]
pub struct CanopyDBBackend {
    /// Optimistric transaction database
    env: Environment,
    /// Map of table names to databases
    dbs: Arc<HashMap<&'static str, canopydb::Database>>,
}

impl CanopyDBBackend {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let environment = Environment::with_options(EnvOptions::new(path)).unwrap();

        let mut dbs = HashMap::with_capacity(TABLES.len());

        for table in TABLES {
            let opts = DbOptions::new();
            let db = environment
                .get_or_create_database_with(table, opts)
                .unwrap();
            let write_tx = db.begin_write().unwrap();
            // We have a single tree on each database
            write_tx
                .get_or_create_tree_with(&[], TreeOptions::default())
                .unwrap();
            write_tx.commit().unwrap();
            dbs.insert(table, db);
        }
        Ok(Self {
            env: environment,
            dbs: Arc::new(dbs),
        })
    }
}

impl StorageBackend for CanopyDBBackend {
    fn clear_table(&self, table: &'static str) -> Result<(), StoreError> {
        let db = self
            .dbs
            .get(table)
            .ok_or_else(|| StoreError::Custom("Column family not found".to_string()))?;

        let write_tx = db.begin_write().unwrap();
        write_tx.get_tree(&[]).unwrap().unwrap().clear().unwrap();
        write_tx.commit().unwrap();
        Ok(())
    }

    fn begin_read(&self) -> Result<Box<dyn StorageReadTx + '_>, StoreError> {
        Ok(Box::new(CanopyDBReadTx {
            dbs: self.dbs.clone(),
        }))
    }

    fn begin_write(&self) -> Result<Box<dyn StorageWriteTx + 'static>, StoreError> {
        Ok(Box::new(CanopyDBWriteTx {
            env: self.env.clone(),
            dbs: self.dbs.clone(),
            write_txs: Default::default(),
        }))
    }

    fn begin_locked(&self, table_name: &'static str) -> Result<Box<dyn StorageLocked>, StoreError> {
        let db = self.dbs.get(table_name).unwrap();
        // The transaction takes a snapshot of the database
        // TODO: please remove this hack
        //   The mutex is needed because CanopyDB ReadTransaction is not Sync,
        //   however, we want to access this concurrently from multiple threads.
        let read_txs = std::iter::repeat_with(|| Mutex::new(db.begin_read().unwrap()))
            .take(16)
            .collect();
        Ok(Box::new(CanopyDBLocked { read_txs }))
    }

    fn create_checkpoint(&self, _path: &Path) -> Result<(), StoreError> {
        todo!()
    }
}

/// Read-only transaction for CanopyDB
pub struct CanopyDBReadTx {
    dbs: Arc<HashMap<&'static str, canopydb::Database>>,
}

impl StorageReadTx for CanopyDBReadTx {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let db = self
            .dbs
            .get(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

        let tx = db.begin_read().unwrap();
        let tree = tx.get_tree(&[]).unwrap().unwrap();
        Ok(tree.get(key).unwrap().map(|b| b.to_vec()))
    }

    fn prefix_iterator(
        &self,
        _table: &'static str,
        _prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError> {
        todo!()
        // let db = self
        //     .dbs
        //     .get(table)
        //     .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

        // let read_tx = Arc::new(db.begin_read().unwrap());
        // let iter = read_tx
        //     .as_ref()
        //     .get_tree(&[])
        //     .unwrap()
        //     .unwrap()
        //     .prefix(&prefix)
        //     .unwrap()
        //     .map(|result| {
        //         let (key, value) = result.unwrap();
        //         Ok((
        //             key.to_vec().into_boxed_slice(),
        //             value.to_vec().into_boxed_slice(),
        //         ))
        //     })
        //     .scan(read_tx.clone(), |_read_tx, item| Some(item));
        // Ok(Box::new(iter))
    }
}

/// Read-write transaction for CanopyDB
pub struct CanopyDBWriteTx {
    /// Database reference for writing
    env: Environment,
    /// Map of table names to databases
    dbs: Arc<HashMap<&'static str, canopydb::Database>>,
    /// Write batch for accumulating changes
    write_txs: HashMap<&'static str, WriteTransaction>,
}

impl StorageReadTx for CanopyDBWriteTx {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self
            .dbs
            .get(table)
            .unwrap()
            .begin_read()
            .unwrap()
            .get_tree(&[])
            .unwrap()
            .unwrap()
            .get(key)
            .unwrap()
            .map(|b| b.to_vec()))
    }

    fn prefix_iterator(
        &self,
        _table: &'static str,
        _prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError> {
        todo!()
        // Ok(Box::new(iter))
    }
}

impl StorageWriteTx for CanopyDBWriteTx {
    /// Stores multiple key-value pairs in different tables using WriteBatch.
    /// Changes are accumulated in the batch and written atomically on commit.
    fn put_batch(
        &mut self,
        table: &'static str,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        // TODO: this modifies the state at current time instead of time of call to begin_write
        let rw_tx = self
            .write_txs
            .entry(table)
            .or_insert_with(|| self.dbs.get(table).unwrap().begin_write().unwrap());

        for (key, value) in batch {
            rw_tx
                .get_tree(&[])
                .unwrap()
                .unwrap()
                .insert(&key, &value)
                .unwrap();
        }
        Ok(())
    }

    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        let rw_tx = self
            .write_txs
            .entry(table)
            .or_insert_with(|| self.dbs.get(table).unwrap().begin_write().unwrap());

        rw_tx.get_tree(&[]).unwrap().unwrap().delete(key).unwrap();
        Ok(())
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        // Take ownership of the batch (replaces it with an empty one) since db.write() consumes it
        let write_txs = std::mem::take(&mut self.write_txs);
        self.env
            .group_commit(write_txs.into_values(), true)
            .unwrap();
        Ok(())
    }
}

/// Locked snapshot for CanopyDB
/// This is used for batch read operations in snap sync
pub struct CanopyDBLocked {
    /// Snapshot/locked transaction
    read_txs: Vec<Mutex<ReadTransaction>>,
}

impl StorageLocked for CanopyDBLocked {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let opt_read_tx = self.read_txs.iter().find_map(|tx| tx.try_lock().ok());
        let read_tx = opt_read_tx
            .or_else(|| Some(self.read_txs.first().unwrap().lock().unwrap()))
            .unwrap();
        Ok(read_tx
            .get_tree(&[])
            .unwrap()
            .unwrap()
            .get(key)
            .unwrap()
            .map(|b| b.to_vec()))
    }
}
