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
    // Each namespace is a separate BTreeMap for ordered key iteration
    namespaces: HashMap<String, Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>>,
}

// Implement RefUnwindSafe manually since Mutex<T> doesn't automatically implement it
impl RefUnwindSafe for InMemoryBackend {}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl StorageBackend for InMemoryBackend {
    fn get_sync(&self, namespace: &str, key: Vec<u8>) -> Result<Option<Vec<u8>>, StoreError> {
        todo!()
    }

    async fn get_async(
        &self,
        namespace: &str,
        key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        todo!()
    }

    async fn get_async_batch(
        &self,
        namespace: &str,
        keys: Vec<Vec<u8>>,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        todo!()
    }

    fn put_sync(&self, namespace: &str, key: Vec<u8>, value: Vec<u8>) -> Result<(), StoreError> {
        todo!()
    }

    async fn put(&self, namespace: &str, key: Vec<u8>, value: Vec<u8>) -> Result<(), StoreError> {
        todo!()
    }

    async fn delete(&self, namespace: &str, key: Vec<u8>) -> Result<(), StoreError> {
        todo!()
    }

    async fn batch_write(&self, ops: Vec<BatchOp>) -> Result<(), StoreError> {
        todo!()
    }

    async fn range(
        &self,
        namespace: &str,
        start_key: Vec<u8>,
        end_key: Option<Vec<u8>>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError> {
        todo!()
    }
}
