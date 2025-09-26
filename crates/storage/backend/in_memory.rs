use crate::api::{
    PrefixResult, StorageBackend, StorageLocked, StorageRoTx, StorageRwTx, TableOptions,
};
use crate::error::StoreError;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

type Table = BTreeMap<Vec<u8>, Vec<u8>>;
type Database = BTreeMap<String, Table>;

#[derive(Debug)]
pub struct InMemoryBackend {
    inner: Arc<RwLock<Database>>,
}

impl StorageBackend for InMemoryBackend {
    fn open(_path: impl AsRef<Path>) -> Result<Self, StoreError>
    where
        Self: Sized,
    {
        Ok(Self {
            inner: Arc::new(RwLock::new(Database::new())),
        })
    }

    fn create_table(&self, name: &str, _options: TableOptions) -> Result<(), StoreError> {
        let mut db = self
            .inner
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire write lock".to_string()))?;

        db.entry(name.to_string()).or_insert_with(Table::new);
        Ok(())
    }

    fn clear_table(&self, table: &str) -> Result<(), StoreError> {
        let mut db = self
            .inner
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire write lock".to_string()))?;

        if let Some(table_ref) = db.get_mut(table) {
            table_ref.clear();
        }
        Ok(())
    }

    fn begin_read(&self) -> Result<Box<dyn StorageRoTx + '_>, StoreError> {
        Ok(Box::new(InMemoryRoTx {
            backend: &self.inner,
        }))
    }

    fn begin_write(&self) -> Result<Box<dyn StorageRwTx + '_>, StoreError> {
        Ok(Box::new(InMemoryRwTx {
            backend: &self.inner,
        }))
    }

    fn begin_locked(&self, table_name: &str) -> Result<Box<dyn StorageLocked>, StoreError> {
        Ok(Box::new(InMemoryLocked {
            backend: self.inner.clone(),
            table_name: table_name.to_string(),
        }))
    }
}

pub struct InMemoryLocked {
    backend: Arc<RwLock<Database>>,
    table_name: String,
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

impl StorageLocked for InMemoryLocked {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let db = self
            .backend
            .read()
            .map_err(|_| StoreError::Custom("Failed to acquire read lock".to_string()))?;
        Ok(db
            .get(&self.table_name)
            .and_then(|table_ref| table_ref.get(key))
            .cloned())
    }
}

pub struct InMemoryRoTx<'a> {
    backend: &'a RwLock<Database>,
}

impl<'a> StorageRoTx for InMemoryRoTx<'a> {
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let db = self
            .backend
            .read()
            .map_err(|_| StoreError::Custom("Failed to acquire read lock".to_string()))?;

        Ok(db
            .get(table)
            .and_then(|table_ref| table_ref.get(key))
            .cloned())
    }

    fn prefix_iterator(
        &self,
        table: &str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<u8>), StoreError>> + '_>, StoreError>
    {
        let db = self
            .backend
            .read()
            .map_err(|_| StoreError::Custom("Failed to acquire read lock".to_string()))?;

        let table_data = db.get(table).cloned().unwrap_or_default();
        let prefix_vec = prefix.to_vec();

        let results: Vec<PrefixResult> = table_data
            .into_iter()
            .filter(|(key, _)| key.starts_with(&prefix_vec))
            .map(|(k, v)| Ok((k, v)))
            .collect();

        let iter = InMemoryPrefixIter {
            results: results.into_iter(),
        };
        Ok(Box::new(iter))
    }
}

pub struct InMemoryRwTx<'a> {
    backend: &'a RwLock<Database>,
}

impl<'a> StorageRoTx for InMemoryRwTx<'a> {
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let db = self
            .backend
            .read()
            .map_err(|_| StoreError::Custom("Failed to acquire read lock".to_string()))?;

        Ok(db
            .get(table)
            .and_then(|table_ref| table_ref.get(key))
            .cloned())
    }

    fn prefix_iterator(
        &self,
        table: &str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<u8>), StoreError>> + '_>, StoreError>
    {
        let db = self
            .backend
            .read()
            .map_err(|_| StoreError::Custom("Failed to acquire read lock".to_string()))?;

        let table_data = db.get(table).cloned().unwrap_or_default();
        let prefix_vec = prefix.to_vec();

        let results: Vec<PrefixResult> = table_data
            .into_iter()
            .filter(|(key, _)| key.starts_with(&prefix_vec))
            .map(|(k, v)| Ok((k, v)))
            .collect();

        let iter = InMemoryPrefixIter {
            results: results.into_iter(),
        };
        Ok(Box::new(iter))
    }
}

impl<'a> StorageRwTx for InMemoryRwTx<'a> {
    fn put_batch(&self, batch: Vec<(&str, Vec<u8>, Vec<u8>)>) -> Result<(), StoreError> {
        let mut db = self
            .backend
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire write lock".to_string()))?;

        for (table, key, value) in batch {
            let table_ref = db.entry(table.to_string()).or_insert_with(Table::new);
            table_ref.insert(key, value);
        }

        Ok(())
    }

    fn delete(&self, table: &str, key: &[u8]) -> Result<(), StoreError> {
        let mut db = self
            .backend
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire write lock".to_string()))?;

        if let Some(table_ref) = db.get_mut(table) {
            table_ref.remove(key);
        }
        Ok(())
    }

    fn commit(self: Box<Self>) -> Result<(), StoreError> {
        // We don't need to commit for in-memory backend
        Ok(())
    }
}
