use super::{BatchOp, StorageBackend, StorageError};
use std::collections::{BTreeMap, HashMap};
use std::panic::RefUnwindSafe;
use std::sync::{Arc, Mutex};

/// Map of namespaces to their key-value pairs
pub type NamespaceMap = HashMap<String, BTreeMap<Vec<u8>, Vec<u8>>>;

/// In-memory storage backend implementation
///
/// This is the simplest possible implementation of StorageBackend.
/// It stores everything in HashMaps in memory, providing a baseline
/// for testing and development.
#[derive(Debug, Clone, Default)]
pub struct InMemoryBackend {
    // Each namespace is a separate BTreeMap for ordered key iteration
    namespaces: Arc<Mutex<NamespaceMap>>,
}

// Implement RefUnwindSafe manually since Mutex<T> doesn't automatically implement it
impl RefUnwindSafe for InMemoryBackend {}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure_namespace_exists(&self, namespace: &str) -> Result<(), StorageError> {
        let mut namespaces = self
            .namespaces
            .lock()
            .map_err(|_| StorageError::Custom("Failed to acquire lock".to_string()))?;

        if !namespaces.contains_key(namespace) {
            namespaces.insert(namespace.to_string(), BTreeMap::new());
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl StorageBackend for InMemoryBackend {
    async fn get(&self, namespace: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        let namespaces = self
            .namespaces
            .lock()
            .map_err(|_| StorageError::Custom("Failed to acquire lock".to_string()))?;

        let ns = namespaces.get(namespace);

        match ns {
            Some(ns) => Ok(ns.get(key).cloned()),
            None => Ok(None),
        }
    }

    async fn put(&self, namespace: &str, key: &[u8], value: &[u8]) -> Result<(), StorageError> {
        let mut namespaces = self
            .namespaces
            .lock()
            .map_err(|_| StorageError::Custom("Failed to acquire lock".to_string()))?;

        let ns = namespaces
            .entry(namespace.to_string())
            .or_insert_with(BTreeMap::new);
        ns.insert(key.to_vec(), value.to_vec());

        Ok(())
    }

    async fn delete(&self, namespace: &str, key: &[u8]) -> Result<(), StorageError> {
        let mut namespaces = self
            .namespaces
            .lock()
            .map_err(|_| StorageError::Custom("Failed to acquire lock".to_string()))?;

        if let Some(ns) = namespaces.get_mut(namespace) {
            ns.remove(key);
        }

        Ok(())
    }

    async fn batch_write(&self, ops: Vec<BatchOp>) -> Result<(), StorageError> {
        let mut namespaces = self
            .namespaces
            .lock()
            .map_err(|_| StorageError::Custom("Failed to acquire lock".to_string()))?;

        // Execute all operations atomically
        for op in ops {
            match op {
                BatchOp::Put {
                    namespace,
                    key,
                    value,
                } => {
                    let ns = namespaces.entry(namespace).or_insert_with(BTreeMap::new);
                    ns.insert(key, value);
                }
                BatchOp::Delete { namespace, key } => {
                    if let Some(ns) = namespaces.get_mut(&namespace) {
                        ns.remove(&key);
                    }
                }
            }
        }

        Ok(())
    }

    async fn init_namespace(&self, namespace: &str) -> Result<(), StorageError> {
        self.ensure_namespace_exists(namespace)
    }

    async fn range(
        &self,
        namespace: &str,
        start_key: &[u8],
        end_key: Option<&[u8]>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StorageError> {
        let namespaces = self
            .namespaces
            .lock()
            .map_err(|_| StorageError::Custom("Failed to acquire lock".to_string()))?;

        let ns = match namespaces.get(namespace) {
            Some(ns) => ns,
            None => return Ok(Vec::new()),
        };

        let mut result = Vec::new();

        for (key, value) in ns.range(start_key.to_vec()..) {
            // Check if we've exceeded the end key
            if let Some(end) = end_key {
                if key.as_slice() >= end {
                    break;
                }
            }

            result.push((key.clone(), value.clone()));
        }

        Ok(result)
    }
}
