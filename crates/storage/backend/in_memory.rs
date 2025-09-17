use super::{BatchOp, StorageBackend};
use crate::error::StoreError;
use std::collections::{BTreeMap, HashMap};
use std::panic::RefUnwindSafe;
use std::sync::{Arc, Mutex};

/// In-memory storage backend implementation
///
/// This is the simplest possible implementation of StorageBackend.
/// It stores everything in HashMaps in memory, providing a baseline
/// for testing and development.
#[derive(Debug, Clone, Default)]
pub struct InMemoryBackend {
    namespaces: HashMap<String, Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>>,
}

// Implement RefUnwindSafe manually since Mutex<T> doesn't automatically implement it
impl RefUnwindSafe for InMemoryBackend {}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn get_table(
        &self,
        namespace: &str,
    ) -> Result<Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>, StoreError> {
        let table = self
            .namespaces
            .get(namespace)
            .ok_or(StoreError::Custom(format!(
                "Namespace not found: {}",
                namespace
            )))?;
        Ok(table.clone())
    }
}

#[async_trait::async_trait]
impl StorageBackend for InMemoryBackend {
    fn get_sync(&self, namespace: &str, key: Vec<u8>) -> Result<Option<Vec<u8>>, StoreError> {
        let table = self.get_table(namespace)?;
        Ok(table
            .lock()
            .map_err(|_| StoreError::LockError)?
            .get(&key)
            .cloned())
    }

    async fn get_async(
        &self,
        namespace: &str,
        key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        self.get_sync(namespace, key)
    }

    async fn get_async_batch(
        &self,
        namespace: &str,
        keys: Vec<Vec<u8>>,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let table = self.get_table(namespace)?;
        let mut values = Vec::new();
        for key in keys {
            let Some(value) = table
                .lock()
                .map_err(|_| StoreError::LockError)?
                .get(&key)
                .cloned()
            else {
                return Err(StoreError::Custom("Key not found".to_string()));
            };
            values.push(value);
        }
        Ok(values)
    }

    fn put_sync(&self, namespace: &str, key: Vec<u8>, value: Vec<u8>) -> Result<(), StoreError> {
        let table = self.get_table(namespace)?;
        table
            .lock()
            .map_err(|_| StoreError::LockError)?
            .insert(key, value);
        Ok(())
    }

    async fn put(&self, namespace: &str, key: Vec<u8>, value: Vec<u8>) -> Result<(), StoreError> {
        self.put_sync(namespace, key, value)
    }

    async fn delete(&self, namespace: &str, key: Vec<u8>) -> Result<(), StoreError> {
        let table = self.get_table(namespace)?;
        table
            .lock()
            .map_err(|_| StoreError::LockError)?
            .remove(&key);
        Ok(())
    }

    async fn batch_write(&self, ops: Vec<BatchOp>) -> Result<(), StoreError> {
        for op in ops {
            match op {
                BatchOp::Put {
                    namespace,
                    key,
                    value,
                } => self.put(&namespace, key, value).await?,
                BatchOp::Delete { namespace, key } => self.delete(&namespace, key).await?,
            }
        }
        Ok(())
    }

    async fn range(
        &self,
        namespace: &str,
        start_key: Vec<u8>,
        end_key: Option<Vec<u8>>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError> {
        let mut values = Vec::new();
        for (key, value) in self
            .get_table(namespace)?
            .lock()
            .map_err(|_| StoreError::LockError)?
            .range(start_key..end_key.unwrap_or_default())
        {
            values.push((key.clone(), value.clone()));
        }
        Ok(values)
    }
}
