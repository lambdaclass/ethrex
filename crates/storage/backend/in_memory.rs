use crate::api::{
    PrefixIterator, StorageBackend, StorageLocked, StorageRoTx, StorageRwTx, TableOptions,
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
    fn open(_path: impl AsRef<Path>) -> Result<Arc<dyn StorageBackend>, StoreError>
    where
        Self: Sized,
    {
        Ok(Arc::new(Self {
            inner: Arc::new(RwLock::new(Database::new())),
        }))
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

    fn begin_read<'a>(&'a self) -> Result<Box<dyn StorageRoTx<'a> + 'a>, StoreError> {
        Ok(Box::new(InMemoryRoTx {
            backend: &self.inner,
        }))
    }

    fn begin_write<'a>(&'a self) -> Result<Box<dyn StorageRwTx<'a> + 'a>, StoreError> {
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

struct InMemoryLocked {
    backend: Arc<RwLock<Database>>,
    table_name: String,
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

impl<'a> StorageRoTx<'a> for InMemoryRoTx<'a> {
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

    fn prefix_iterator(&self, table: &str, prefix: &[u8]) -> Result<PrefixIterator, StoreError> {
        let db = self
            .backend
            .read()
            .map_err(|_| StoreError::Custom("Failed to acquire read lock".to_string()))?;

        let table_data = db.get(table).cloned().unwrap_or_default();
        let prefix_vec = prefix.to_vec();

        // Crear un iterador que filtra por prefix
        let iter = table_data
            .into_iter()
            .filter(move |(key, _)| key.starts_with(&prefix_vec))
            .map(|(k, v)| Ok((k, v)));

        Ok(Box::new(iter))
    }
}

pub struct InMemoryRwTx<'a> {
    backend: &'a RwLock<Database>,
}

impl<'a> StorageRwTx<'a> for InMemoryRwTx<'a> {
    fn put(&self, table: &str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        let mut db = self
            .backend
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire write lock".to_string()))?;

        let table_ref = db.entry(table.to_string()).or_insert_with(Table::new);

        table_ref.insert(key.to_vec(), value.to_vec());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_operations() {
        let backend = InMemoryBackend::open("").unwrap();
        backend
            .create_table("test", TableOptions { dupsort: false })
            .unwrap();

        // Test write transaction
        {
            let tx = backend.begin_write().unwrap();
            tx.put("test", b"key1", b"value1").unwrap();
            tx.put("test", b"key2", b"value2").unwrap();
            tx.commit().unwrap();
        }

        // Test read transaction
        {
            let tx = backend.begin_read().unwrap();
            assert_eq!(tx.get("test", b"key1").unwrap(), Some(b"value1".to_vec()));
            assert_eq!(tx.get("test", b"key2").unwrap(), Some(b"value2".to_vec()));
            assert_eq!(tx.get("test", b"nonexistent").unwrap(), None);
        }
    }

    #[test]
    fn test_prefix_iterator() {
        let backend = InMemoryBackend::open("").unwrap();
        backend
            .create_table("test", TableOptions { dupsort: false })
            .unwrap();

        // Insert test data
        {
            let tx = backend.begin_write().unwrap();
            tx.put("test", b"prefix_key1", b"value1").unwrap();
            tx.put("test", b"prefix_key2", b"value2").unwrap();
            tx.put("test", b"other_key", b"value3").unwrap();
            tx.commit().unwrap();
        }

        // Test prefix iterator
        {
            let tx = backend.begin_read().unwrap();
            let iter = tx.prefix_iterator("test", b"prefix_").unwrap();
            let results: Result<Vec<_>, _> = iter.collect();
            let results = results.unwrap();

            assert_eq!(results.len(), 2);
            // BTreeMap mantiene orden lexicográfico
            assert_eq!(results[0], (b"prefix_key1".to_vec(), b"value1".to_vec()));
            assert_eq!(results[1], (b"prefix_key2".to_vec(), b"value2".to_vec()));
        }
    }

    #[test]
    fn test_immediate_writes() {
        let backend = InMemoryBackend::open("").unwrap();
        backend
            .create_table("test", TableOptions { dupsort: false })
            .unwrap();

        // Writes are immediately visible (no transaction isolation)
        {
            let tx1 = backend.begin_write().unwrap();
            tx1.put("test", b"key1", b"value1").unwrap();
            tx1.commit().unwrap();
        }

        // Read should see the changes immediately
        {
            let tx2 = backend.begin_read().unwrap();
            assert_eq!(tx2.get("test", b"key1").unwrap(), Some(b"value1".to_vec()));
        }
    }

    #[test]
    fn test_delete_operations() {
        let backend = InMemoryBackend::open("").unwrap();
        backend
            .create_table("test", TableOptions { dupsort: false })
            .unwrap();

        // Insert and then delete
        {
            let tx = backend.begin_write().unwrap();
            tx.put("test", b"key1", b"value1").unwrap();
            tx.commit().unwrap();
        }

        {
            let tx = backend.begin_write().unwrap();
            tx.delete("test", b"key1").unwrap();
            tx.commit().unwrap();
        }

        // Verify deletion
        {
            let tx = backend.begin_read().unwrap();
            assert_eq!(tx.get("test", b"key1").unwrap(), None);
        }
    }
}
