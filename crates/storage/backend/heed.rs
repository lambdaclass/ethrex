use crate::api::{
    PrefixResult, StorageBackend, StorageLockedView, StorageReadView, StorageWriteBatch,
    tables::TABLES,
};
use crate::error::StoreError;
use heed3::types::Bytes;
use heed3::{Database, DatabaseFlags, DatabaseOpenOptions, EnvFlags, EnvOpenOptions, WithoutTls};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

// We don't use TLS since we need HeedLocked to be Send + Sync
// TODO: this should be easy to change
type Env = heed3::Env<WithoutTls>;

/// Heed backend
#[derive(Debug)]
pub struct HeedBackend {
    /// Optimistric transaction database
    env: Env,
    dbs: Arc<HashMap<&'static str, Database<Bytes, Bytes>>>,
}

impl HeedBackend {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let mut env_opts = EnvOpenOptions::new().read_txn_without_tls();
        env_opts.max_dbs(TABLES.len().try_into().unwrap());
        env_opts.map_size(4 * 1024 * 1024 * 1024 * 1024); // 4 TB

        unsafe {
            env_opts.flags(EnvFlags::NO_META_SYNC | EnvFlags::WRITE_MAP | EnvFlags::MAP_ASYNC)
        };

        let env = unsafe { env_opts.open(path) }.unwrap();

        // We open the default unnamed database
        let mut wtxn = env.write_txn().unwrap();

        let mut dbs = HashMap::new();
        for cf_name in TABLES {
            let mut opts = DatabaseOpenOptions::new(&env).types::<Bytes, Bytes>();
            opts.name(cf_name);
            let db: Database<Bytes, Bytes> = opts.create(&mut wtxn).unwrap();
            dbs.insert(cf_name, db);
        }
        wtxn.commit().unwrap();

        Ok(Self {
            env,
            dbs: Arc::new(dbs),
        })
    }
}

impl StorageBackend for HeedBackend {
    fn clear_table(&self, table: &'static str) -> Result<(), StoreError> {
        let db = self
            .dbs
            .get(table)
            .ok_or_else(|| StoreError::Custom("Column family not found".to_string()))?;

        let mut wtxn = self.env.write_txn().unwrap();

        db.clear(&mut wtxn).unwrap();
        Ok(())
    }

    fn begin_read(&self) -> Result<Box<dyn StorageReadView + '_>, StoreError> {
        let env = Box::leak(Box::new(self.env.clone()));
        let rtxn = env.read_txn().unwrap();
        Ok(Box::new(HeedReadTx {
            env,
            dbs: self.dbs.clone(),
            rtxn,
        }))
    }

    fn begin_write(&self) -> Result<Box<dyn StorageWriteBatch + 'static>, StoreError> {
        let env = Box::leak(Box::new(self.env.clone()));
        let wtxn = env.write_txn().unwrap();
        Ok(Box::new(HeedWriteTx {
            env,
            dbs: self.dbs.clone(),
            wtxn: Some(wtxn),
        }))
    }

    fn begin_locked(
        &self,
        table_name: &'static str,
    ) -> Result<Box<dyn StorageLockedView>, StoreError> {
        let env = Box::leak(Box::new(self.env.clone()));
        let rtxn = env.read_txn().unwrap();
        Ok(Box::new(HeedLocked {
            env,
            dbs: self.dbs.clone(),
            rtxn: Mutex::new(rtxn),
            table_name,
        }))
    }

    fn create_checkpoint(&self, path: &Path) -> Result<(), StoreError> {
        todo!()
    }
}

/// Read-only view for Heed
pub struct HeedReadTx {
    env: &'static Env,
    dbs: Arc<HashMap<&'static str, Database<Bytes, Bytes>>>,
    rtxn: heed3::RoTxn<'static, WithoutTls>,
}

impl Drop for HeedReadTx {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.env as *const Env as *mut Env));
        }
    }
}

impl StorageReadView for HeedReadTx {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let db = self
            .dbs
            .get(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

        // LMDB does not support empty keys, so we prepend a zero byte
        let key = [&[0], key].concat();

        let value = db.get(&self.rtxn, &key).unwrap().map(|b| b.to_vec());
        Ok(value)
    }

    fn prefix_iterator(
        &self,
        table: &'static str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError> {
        todo!("serving snapsync is a WIP")
        // let cf = self
        //     .db
        //     .cf_handle(table)
        //     .ok_or_else(|| StoreError::Custom(format!("Table {} not found", table)))?;

        // let iter = self.db.prefix_iterator_cf(&cf, prefix).map(|result| {
        //     result.map_err(|e| StoreError::Custom(format!("Failed to iterate: {e}")))
        // });
        // Ok(Box::new(iter))
    }
}

/// Write batch for Heed
pub struct HeedWriteTx {
    env: &'static Env,
    dbs: Arc<HashMap<&'static str, Database<Bytes, Bytes>>>,
    wtxn: Option<heed3::RwTxn<'static>>,
}

impl Drop for HeedWriteTx {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.env as *const Env as *mut Env));
        }
    }
}

impl StorageWriteBatch for HeedWriteTx {
    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        let db = self
            .dbs
            .get(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {table:?} not found")))?;

        // LMDB does not support empty keys, so we prepend a zero byte
        let key = [&[0], key].concat();
        db.put(self.wtxn.as_mut().unwrap(), &key, value).unwrap();
        Ok(())
    }

    /// Stores multiple key-value pairs in a single table.
    /// Changes are accumulated in the batch and written atomically on commit.
    fn put_batch(
        &mut self,
        table: &'static str,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        let db = self
            .dbs
            .get(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {table:?} not found")))?;

        for (mut key, value) in batch {
            // LMDB does not support empty keys, so we prepend a zero byte
            key.insert(0, 0);
            db.put(self.wtxn.as_mut().unwrap(), &key, &value).unwrap();
        }
        Ok(())
    }

    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        let db = self
            .dbs
            .get(table)
            .ok_or_else(|| StoreError::Custom(format!("Table {table:?} not found")))?;

        // LMDB does not support empty keys, so we prepend a zero byte
        let key = [&[0], key].concat();
        db.delete(self.wtxn.as_mut().unwrap(), &key).unwrap();
        Ok(())
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        // Take ownership of the batch (replaces it with an empty one) since db.write() consumes it
        self.wtxn.take().unwrap().commit().unwrap();
        Ok(())
    }
}

/// Locked snapshot for Heed
/// This is used for batch read operations in snap sync
pub struct HeedLocked {
    env: &'static Env,
    dbs: Arc<HashMap<&'static str, Database<Bytes, Bytes>>>,
    rtxn: Mutex<heed3::RoTxn<'static, WithoutTls>>,
    table_name: &'static str,
}

impl StorageLockedView for HeedLocked {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        // LMDB does not support empty keys, so we prepend a zero byte
        let key = [&[0], key].concat();
        let db = self
            .dbs
            .get(self.table_name)
            .ok_or_else(|| StoreError::Custom(format!("Table {} not found", self.table_name)))?;
        let rtxn = self.rtxn.lock().unwrap();
        let value = db.get(&rtxn, &key).unwrap().map(|b| b.to_vec());
        Ok(value)
    }
}

impl Drop for HeedLocked {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.env as *const Env as *mut Env));
        }
    }
}
