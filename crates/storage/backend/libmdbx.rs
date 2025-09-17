use std::{collections::HashMap, sync::Arc};

use libmdbx::{
    Database, DatabaseOptions, Mode, PageSize, RW, ReadWriteOptions, TableFlags, Transaction,
    WriteFlags, WriteMap,
};

use super::{BatchOp, StorageBackend};
use crate::{engine::DBTable, error::StoreError};

/// default page size recommended by libmdbx
///
/// - See here: https://github.com/erthink/libmdbx/tree/master?tab=readme-ov-file#limitations
/// - and here: https://libmdbx.dqdkfa.ru/structmdbx_1_1env_1_1geometry.html#a45048bf2de9120d01dae2151c060d459
const DB_PAGE_SIZE: usize = 4096;
/// For a default page size of 4096, the max value size is roughly 1/2 page size.
const DB_MAX_VALUE_SIZE: usize = 2022;
// Maximum DB size, set to 8 TB
const MAX_MAP_SIZE: isize = 1024_isize.pow(4) * 8; // 8 TB

/// LibMDBX storage backend implementation
///
/// This adapter translates generic storage operations to LibMDBX tables.
/// Each namespace corresponds to a LibMDBX table.
#[derive(Debug)]
pub struct LibmdbxBackend {
    db: Arc<Database<WriteMap>>,
}

impl LibmdbxBackend {
    pub fn new(path: &str) -> Result<Self, StoreError> {
        // TODO: Add dupsort when needed
        let options = DatabaseOptions {
            page_size: Some(PageSize::Set(DB_PAGE_SIZE)),
            mode: Mode::ReadWrite(ReadWriteOptions {
                max_size: Some(MAX_MAP_SIZE),
                ..Default::default()
            }),
            ..Default::default()
        };
        let db = Database::open_with_options(path, options)
            .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
        let txn = db
            .begin_rw_txn()
            .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;

        let table_names = DBTable::all()
            .iter()
            .map(|table| table.namespace())
            .collect::<Vec<_>>();

        let mut tables = HashMap::new();
        for table_name in table_names {
            let table = txn
                .create_table(Some(table_name), TableFlags::default())
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            tables.insert(table_name.to_string(), table);
        }

        txn.commit()
            .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;

        Ok(Self { db: Arc::new(db) })
    }

    async fn execute_batch<F>(&self, operations: F) -> Result<(), StoreError>
    where
        F: FnOnce(&Transaction<RW, WriteMap>) -> Result<(), StoreError> + Send + 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db
                .begin_rw_txn()
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;

            operations(&txn)?;

            txn.commit()
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }
}

#[async_trait::async_trait]
impl StorageBackend for LibmdbxBackend {
    fn get_sync(&self, namespace: &str, key: Vec<u8>) -> Result<Option<Vec<u8>>, StoreError> {
        let txn = self
            .db
            .begin_ro_txn()
            .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
        let table = txn
            .open_table(Some(namespace))
            .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
        let value = txn
            .get(&table, &key)
            .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
        Ok(value)
    }

    async fn get_async(
        &self,
        namespace: &str,
        key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let db = self.db.clone();
        let namespace = namespace.to_string();
        tokio::task::spawn_blocking(move || {
            let txn = db
                .begin_ro_txn()
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            let table = txn
                .open_table(Some(&namespace))
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            let value = txn
                .get(&table, &key)
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            Ok(value)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }

    async fn get_async_batch(
        &self,
        namespace: &str,
        keys: Vec<Vec<u8>>,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let db = self.db.clone();
        let namespace = namespace.to_string();
        tokio::task::spawn_blocking(move || {
            let mut res = Vec::new();
            let txn = db
                .begin_ro_txn()
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            let table = txn
                .open_table(Some(&namespace))
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            for key in keys {
                let val = txn
                    .get(&table, &key)
                    .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
                match val {
                    Some(val) => res.push(val),
                    None => Err(StoreError::ReadError)?,
                }
            }
            Ok(res)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    fn put_sync(&self, namespace: &str, key: Vec<u8>, value: Vec<u8>) -> Result<(), StoreError> {
        let txn = self
            .db
            .begin_rw_txn()
            .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
        let table = txn
            .open_table(Some(namespace))
            .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
        txn.put(&table, &key, &value, WriteFlags::default())
            .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
        txn.commit()
            .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
        Ok(())
    }

    async fn put(&self, namespace: &str, key: Vec<u8>, value: Vec<u8>) -> Result<(), StoreError> {
        let db = self.db.clone();
        let namespace = namespace.to_string();
        tokio::task::spawn_blocking(move || {
            let txn = db
                .begin_rw_txn()
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            let table = txn
                .open_table(Some(&namespace))
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            txn.put(&table, &key, &value, WriteFlags::default())
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            txn.commit()
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn delete(&self, namespace: &str, key: Vec<u8>) -> Result<(), StoreError> {
        let db = self.db.clone();
        let namespace = namespace.to_string();
        tokio::task::spawn_blocking(move || {
            let txn = db
                .begin_rw_txn()
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            let table = txn
                .open_table(Some(&namespace))
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            txn.del(&table, &key, None)
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            txn.commit()
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn batch_write(&self, ops: Vec<BatchOp>) -> Result<(), StoreError> {
        self.execute_batch(move |txn| {
            for op in ops {
                match op {
                    BatchOp::Put {
                        namespace,
                        key,
                        value,
                    } => {
                        let table = txn
                            .open_table(Some(&namespace))
                            .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
                        txn.put(&table, &key, &value, WriteFlags::default())
                            .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
                    }
                    BatchOp::Delete { namespace, key } => {
                        let table = txn
                            .open_table(Some(&namespace))
                            .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
                        txn.del(&table, &key, None)
                            .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
                    }
                }
            }
            Ok(())
        })
        .await
    }

    async fn range(
        &self,
        namespace: &str,
        start_key: Vec<u8>,
        end_key: Option<Vec<u8>>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError> {
        let db = self.db.clone();
        let namespace = namespace.to_string();

        tokio::task::spawn_blocking(move || {
            let txn = db
                .begin_ro_txn()
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            let table = txn
                .open_table(Some(&namespace))
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;

            let mut result = Vec::new();

            let mut cursor = txn
                .cursor(&table)
                .map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;
            let iter = cursor.iter_from(&start_key);

            for item in iter {
                let (key, value): (Vec<u8>, Vec<u8>) =
                    item.map_err(|e| StoreError::LibmdbxError(anyhow::anyhow!(e)))?;

                if let Some(ref end) = end_key {
                    if key >= *end {
                        break;
                    }
                }

                result.push((key.to_vec(), value.to_vec()));
            }

            Ok(result)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("Task panicked: {}", e)))?
    }
}
