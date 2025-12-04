#![expect(clippy::unwrap_used)]
use crate::{
    api::{StorageBackend, StorageLocked, StorageReadTx, StorageWriteTx, tables::TABLES},
    error::StoreError,
};
use fjall::{Config, Keyspace, PartitionCreateOptions, PartitionHandle, Snapshot, WriteBatch};
use std::{fmt::Debug, path::Path};

impl Debug for FjallBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FjallBackend").finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct FjallBackend {
    keyspace: Keyspace,
}
impl FjallBackend {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let keyspace = Config::new(path)
            .max_write_buffer_size(10_u64.pow(9))
            // .fsync_ms(Some(500)
            .max_open_files(16000)
            .open()
            .unwrap();

        // Initialize all partitions
        for table in TABLES {
            init_partition(table, &keyspace).unwrap();
        }

        Ok(Self { keyspace })
    }
}

impl StorageBackend for FjallBackend {
    fn clear_table(&self, table: &'static str) -> Result<(), StoreError> {
        let handle = self.get_partition(table)?;
        self.keyspace.delete_partition(handle).unwrap();
        init_partition(table, &self.keyspace)?;
        Ok(())
    }

    fn begin_read(&self) -> Result<Box<dyn crate::api::StorageReadTx + '_>, StoreError> {
        let backend = self.clone();
        Ok(Box::new(FjallReadTx { backend }))
    }

    fn begin_write(&self) -> Result<Box<dyn crate::api::StorageWriteTx + 'static>, StoreError> {
        let backend = self.clone();
        let write_batch = self.keyspace.batch();
        Ok(Box::new(FjallWriteTx {
            backend,
            write_batch,
        }))
    }

    fn begin_locked(
        &self,
        table_name: &'static str,
    ) -> Result<Box<dyn crate::api::StorageLocked>, StoreError> {
        let snapshot = self.get_partition(table_name)?.snapshot();
        Ok(Box::new(FjallLockedTx { snapshot }))
    }

    fn create_checkpoint(&self, _path: &Path) -> Result<(), StoreError> {
        // TODO: this is use in the L2
        todo!()
    }
}

// Helper method to initialize a single partition
fn init_partition(table_name: &str, keyspace: &Keyspace) -> Result<PartitionHandle, StoreError> {
    let opts = PartitionCreateOptions::default().max_memtable_size(64 * 1024 * 1024);
    let partition = keyspace.open_partition(table_name, opts).unwrap();
    Ok(partition)
}

impl FjallBackend {
    fn get_partition(&self, table_name: &str) -> Result<PartitionHandle, StoreError> {
        // Use default opts, since partition should already exist
        let opts = PartitionCreateOptions::default();
        let partition = self.keyspace.open_partition(table_name, opts).unwrap();
        Ok(partition)
    }
}

struct FjallReadTx {
    backend: FjallBackend,
}

impl StorageReadTx for FjallReadTx {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let partition = self.backend.get_partition(table).unwrap();
        // NOTE: we prepend a 0 to avoid keys being empty, since that triggers a panic in put_batch
        let mut key = key.to_vec();
        key.insert(0, 0);
        let value = partition.get(&key).unwrap();
        Ok(value.map(|v| v.to_vec()))
    }

    fn prefix_iterator(
        &self,
        table: &'static str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = crate::api::PrefixResult> + '_>, StoreError> {
        let partition = self.backend.get_partition(table).unwrap();
        let mut prefix = prefix.to_vec();
        prefix.insert(0, 0);
        Ok(Box::new(partition.prefix(&prefix).map(|res| {
            let (k, v) = res.unwrap();
            Ok((
                k[1..].to_vec().into_boxed_slice(),
                v.to_vec().into_boxed_slice(),
            ))
        })))
    }
}

struct FjallWriteTx {
    backend: FjallBackend,
    write_batch: WriteBatch,
}

impl StorageWriteTx for FjallWriteTx {
    fn put_batch(
        &mut self,
        table: &'static str,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        let mut partitions = std::collections::HashMap::new();
        for (mut key, value) in batch {
            let partition = partitions
                .entry(table)
                .or_insert_with(|| self.backend.get_partition(table).unwrap());
            // NOTE: we prepend a 0 to avoid keys being empty, since that triggers a panic
            key.insert(0, 0);
            self.write_batch.insert(partition, key, value);
        }
        Ok(())
    }

    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        let partition = self.backend.get_partition(table).unwrap();
        // NOTE: we prepend a 0 to avoid keys being empty, since that triggers a panic in put_batch
        let mut key = key.to_vec();
        key.insert(0, 0);
        partition.remove(key).unwrap();
        Ok(())
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        let empty_batch = self.backend.keyspace.batch();
        std::mem::replace(&mut self.write_batch, empty_batch)
            .commit()
            .unwrap();
        Ok(())
    }
}

struct FjallLockedTx {
    snapshot: Snapshot,
}

impl StorageLocked for FjallLockedTx {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        // NOTE: we prepend a 0 to avoid keys being empty, since that triggers a panic in put_batch
        let mut key = key.to_vec();
        key.insert(0, 0);
        Ok(self.snapshot.get(key).unwrap().map(|v| v.to_vec()))
    }
}
