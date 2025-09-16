use rocksdb::{
    ColumnFamilyDescriptor, DBWithThreadMode, Direction, IteratorMode, MultiThreaded, Options,
    WriteBatch,
};

use crate::v2::schema::DBTable;

use super::{BatchOp, StorageBackend, StorageError};
use std::sync::Arc;

/// RocksDB storage backend implementation
///
/// This adapter translates generic storage operations to RocksDB column families.
/// Each namespace corresponds to a RocksDB column family
#[derive(Debug)]
pub struct RocksDBBackend {
    db: Arc<DBWithThreadMode<MultiThreaded>>,
}

impl RocksDBBackend {
    pub fn new(path: &str) -> Result<Self, StorageError> {
        let options = Options::default();
        let tables = DBTable::all()
            .iter()
            .map(|table| ColumnFamilyDescriptor::new(table.namespace(), Options::default()))
            .collect::<Vec<_>>();

        let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(&options, path, tables)
            .map_err(|e| StorageError::Custom(format!("Failed to open RocksDB: {}", e)))?;

        Ok(Self { db: Arc::new(db) })
    }
}

#[async_trait::async_trait]
impl StorageBackend for RocksDBBackend {
    fn get_sync(&self, namespace: &str, key: Vec<u8>) -> Result<Option<Vec<u8>>, StorageError> {
        let cf = self.db.cf_handle(namespace).ok_or_else(|| {
            StorageError::Custom(format!("Column family not found: {}", namespace))
        })?;

        self.db.get_cf(&cf, key).map_err(StorageError::from)
    }

    async fn get_async(
        &self,
        namespace: &str,
        key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        let db = self.db.clone();
        let namespace = namespace.to_string();
        tokio::task::spawn_blocking(move || {
            let cf = db.cf_handle(&namespace).ok_or_else(|| {
                StorageError::Custom(format!("Column family not found: {}", namespace))
            })?;

            db.get_cf(&cf, &key).map_err(StorageError::from)
        })
        .await
        .map_err(|e| StorageError::Custom(format!("Task panicked: {}", e)))?
    }

    async fn get_async_batch(
        &self,
        namespace: &str,
        keys: Vec<Vec<u8>>,
    ) -> Result<Vec<Vec<u8>>, StorageError> {
        let db = self.db.clone();
        let namespace = namespace.to_string();

        tokio::task::spawn_blocking(move || {
            let cf = db.cf_handle(&namespace).ok_or_else(|| {
                StorageError::Custom(format!("Column family not found: {}", namespace))
            })?;

            // Prepare keys with column family references
            let keys_with_cf: Vec<_> = keys.iter().map(|k| (&cf, k.as_slice())).collect();

            // Use multi_get_cf for efficient batch operation
            let results = db.multi_get_cf(keys_with_cf);

            let mut values = Vec::new();
            for result in results {
                match result.map_err(StorageError::from)? {
                    Some(value) => values.push(value),
                    None => {
                        return Err(StorageError::Custom(
                            "Key not found in bulk read".to_string(),
                        ));
                    }
                }
            }

            Ok(values)
        })
        .await
        .map_err(|e| StorageError::Custom(format!("Task panicked: {}", e)))?
    }

    fn put_sync(&self, namespace: &str, key: Vec<u8>, value: Vec<u8>) -> Result<(), StorageError> {
        let db = self.db.clone();
        let namespace = namespace.to_string();
        let cf = db.cf_handle(&namespace).ok_or_else(|| {
            StorageError::Custom(format!("Column family not found: {}", namespace))
        })?;
        db.put_cf(&cf, &key, &value).map_err(StorageError::from)
    }

    async fn put(&self, namespace: &str, key: Vec<u8>, value: Vec<u8>) -> Result<(), StorageError> {
        let db = self.db.clone();
        let namespace = namespace.to_string();

        tokio::task::spawn_blocking(move || {
            let cf = db.cf_handle(&namespace).ok_or_else(|| {
                StorageError::Custom(format!("Column family not found: {}", namespace))
            })?;

            db.put_cf(&cf, &key, &value).map_err(StorageError::from)
        })
        .await
        .map_err(|e| StorageError::Custom(format!("Task panicked: {}", e)))?
    }

    async fn delete(&self, namespace: &str, key: Vec<u8>) -> Result<(), StorageError> {
        let db = self.db.clone();
        let namespace = namespace.to_string();

        tokio::task::spawn_blocking(move || {
            let cf = db.cf_handle(&namespace).ok_or_else(|| {
                StorageError::Custom(format!("Column family not found: {}", namespace))
            })?;

            db.delete_cf(&cf, &key).map_err(StorageError::from)
        })
        .await
        .map_err(|e| StorageError::Custom(format!("Task panicked: {}", e)))?
    }

    async fn batch_write(&self, ops: Vec<BatchOp>) -> Result<(), StorageError> {
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            let mut batch = WriteBatch::default();

            for op in ops {
                match op {
                    BatchOp::Put {
                        namespace,
                        key,
                        value,
                    } => {
                        let cf = db.cf_handle(&namespace).ok_or_else(|| {
                            StorageError::Custom(format!("Column family not found: {}", namespace))
                        })?;
                        batch.put_cf(&cf, &key, &value);
                    }
                    BatchOp::Delete { namespace, key } => {
                        let cf = db.cf_handle(&namespace).ok_or_else(|| {
                            StorageError::Custom(format!("Column family not found: {}", namespace))
                        })?;
                        batch.delete_cf(&cf, &key);
                    }
                }
            }

            db.write(batch).map_err(StorageError::from)
        })
        .await
        .map_err(|e| StorageError::Custom(format!("Task panicked: {}", e)))?
    }

    fn init_namespace(&self, namespace: &str) -> Result<(), StorageError> {
        // Column families are already created during DB initialization
        // Just verify the namespace exists
        if self.db.cf_handle(namespace).is_none() {
            return Err(StorageError::Custom(format!(
                "Column family not found: {}",
                namespace
            )));
        }
        Ok(())
    }

    async fn range(
        &self,
        namespace: &str,
        start_key: Vec<u8>,
        end_key: Option<Vec<u8>>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StorageError> {
        let db = self.db.clone();
        let namespace = namespace.to_string();
        let end_key = end_key.map(|k| k.to_vec());

        tokio::task::spawn_blocking(move || {
            let cf = db.cf_handle(&namespace).ok_or_else(|| {
                StorageError::Custom(format!("Column family not found: {}", namespace))
            })?;

            let mut result = Vec::new();
            let iter = db.iterator_cf(&cf, IteratorMode::From(&start_key, Direction::Forward));

            for item in iter {
                let (key, value) = item.map_err(StorageError::from)?;

                // Check if we've exceeded the end key
                if let Some(ref end) = end_key {
                    if key.as_ref() >= end.as_slice() {
                        break;
                    }
                }

                result.push((key.to_vec(), value.to_vec()));
            }

            Ok(result)
        })
        .await
        .map_err(|e| StorageError::Custom(format!("Task panicked: {}", e)))?
    }
}
