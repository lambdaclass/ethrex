use crate::api::{
    PrefixResult, StorageBackend, StorageLockedView, StorageReadView, StorageWriteBatch,
};
use crate::error::StoreError;
use rustc_hash::FxHashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

type Table = FxHashMap<Vec<u8>, Vec<u8>>;
type Database = FxHashMap<&'static str, Table>;

#[derive(Debug)]
pub struct InMemoryBackend {
    // RCU-style snapshot store: readers clone the inner Arc and then read lock-free.
    // Writes run under the outer write lock and use Arc::make_mut for copy-on-write.
    // If read snapshots are still alive, writes may clone the full Database.
    inner: Arc<RwLock<Arc<Database>>>,
}

impl InMemoryBackend {
    pub fn open() -> Result<Self, StoreError> {
        Ok(Self {
            inner: Arc::new(RwLock::new(Arc::new(Database::default()))),
        })
    }
}

impl StorageBackend for InMemoryBackend {
    fn clear_table(&self, table: &str) -> Result<(), StoreError> {
        let mut db = self
            .inner
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire write lock".to_string()))?;

        let db_mut = Arc::make_mut(&mut *db);
        if let Some(table_ref) = db_mut.get_mut(table) {
            table_ref.clear();
        }
        Ok(())
    }

    fn begin_read(&self) -> Result<Arc<dyn StorageReadView>, StoreError> {
        let snapshot = self
            .inner
            .read()
            .map_err(|_| StoreError::Custom("Failed to acquire read lock".to_string()))?
            .clone();
        Ok(Arc::new(InMemoryReadTx { snapshot }))
    }

    fn begin_write(&self) -> Result<Box<dyn StorageWriteBatch + 'static>, StoreError> {
        Ok(Box::new(InMemoryWriteTx {
            backend: self.inner.clone(),
        }))
    }

    fn begin_locked(
        &self,
        table_name: &'static str,
    ) -> Result<Box<dyn StorageLockedView>, StoreError> {
        let snapshot = self
            .inner
            .read()
            .map_err(|_| StoreError::Custom("Failed to acquire read lock".to_string()))?
            .clone();
        Ok(Box::new(InMemoryLocked {
            snapshot,
            table_name,
        }))
    }

    fn create_checkpoint(&self, _path: &Path) -> Result<(), StoreError> {
        // Checkpoints are not supported for the InMemory DB
        // Silently ignoring the request to create a checkpoint is harmless
        Ok(())
    }
}

pub struct InMemoryLocked {
    snapshot: Arc<Database>,
    table_name: &'static str,
}

pub struct InMemoryPrefixIter {
    results: std::vec::IntoIter<PrefixResult>,
}

impl Iterator for InMemoryPrefixIter {
    type Item = PrefixResult;

    fn next(&mut self) -> Option<Self::Item> {
        self.results.next()
    }
}

impl StorageLockedView for InMemoryLocked {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self
            .snapshot
            .get(&self.table_name)
            .and_then(|table_ref| table_ref.get(key))
            .cloned())
    }
}

pub struct InMemoryReadTx {
    snapshot: Arc<Database>,
}

impl StorageReadView for InMemoryReadTx {
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self
            .snapshot
            .get(table)
            .and_then(|table_ref| table_ref.get(key))
            .cloned())
    }

    fn prefix_iterator(
        &self,
        table: &str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError> {
        let table_data = self.snapshot.get(table).cloned().unwrap_or_default();
        let prefix_vec = prefix.to_vec();

        let results: Vec<PrefixResult> = table_data
            .into_iter()
            .filter(|(key, _)| key.starts_with(&prefix_vec))
            .map(|(k, v)| Ok((k.into_boxed_slice(), v.into_boxed_slice())))
            .collect();

        let iter = InMemoryPrefixIter {
            results: results.into_iter(),
        };
        Ok(Box::new(iter))
    }
}

pub struct InMemoryWriteTx {
    backend: Arc<RwLock<Arc<Database>>>,
}

impl StorageWriteBatch for InMemoryWriteTx {
    fn put_batch(
        &mut self,
        table: &'static str,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        let mut db = self
            .backend
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire write lock".to_string()))?;

        // Copy-on-write update of the current snapshot.
        let db_mut = Arc::make_mut(&mut *db);
        let table_ref = db_mut.entry(table).or_default();

        for (key, value) in batch {
            table_ref.insert(key, value);
        }

        Ok(())
    }

    fn delete(&mut self, table: &str, key: &[u8]) -> Result<(), StoreError> {
        let mut db = self
            .backend
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire write lock".to_string()))?;

        let db_mut = Arc::make_mut(&mut *db);
        if let Some(table_ref) = db_mut.get_mut(table) {
            table_ref.remove(key);
        }
        Ok(())
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        // FIXME: in-memory writes aren't atomic
        Ok(())
    }
}
