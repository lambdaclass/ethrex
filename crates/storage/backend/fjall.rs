#![expect(clippy::unwrap_used)]
use crate::{
    api::{StorageBackend, StorageLocked, StorageRoTx, StorageRwTx, tables::TABLES},
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

    fn begin_read(&self) -> Result<Box<dyn crate::api::StorageRoTx + '_>, StoreError> {
        let backend = self.clone();
        Ok(Box::new(FjallRoTx { backend }))
    }

    fn begin_write(&self) -> Result<Box<dyn crate::api::StorageRwTx + 'static>, StoreError> {
        let backend = self.clone();
        let write_batch = self.keyspace.batch();
        let ro_tx = FjallRoTx { backend };
        Ok(Box::new(FjallRwTx { ro_tx, write_batch }))
    }

    fn begin_locked(
        &self,
        table_name: &'static str,
    ) -> Result<Box<dyn crate::api::StorageLocked>, StoreError> {
        let snapshot = self.get_partition(table_name)?.snapshot();
        Ok(Box::new(FjallLockedTx { snapshot }))
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

struct FjallRoTx {
    backend: FjallBackend,
}

impl StorageRoTx for FjallRoTx {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let partition = self.backend.get_partition(table).unwrap();
        let value = partition.get(key).unwrap();
        Ok(value.map(|v| v.to_vec()))
    }

    fn prefix_iterator(
        &self,
        table: &'static str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = crate::api::PrefixResult> + '_>, StoreError> {
        let partition = self.backend.get_partition(table).unwrap();
        Ok(Box::new(partition.prefix(prefix).map(|res| {
            let (k, v) = res.unwrap();
            Ok((k.to_vec().into_boxed_slice(), v.to_vec().into_boxed_slice()))
        })))
    }
}

struct FjallRwTx {
    ro_tx: FjallRoTx,
    write_batch: WriteBatch,
}

impl StorageRoTx for FjallRwTx {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        self.ro_tx.get(table, key)
    }

    fn prefix_iterator(
        &self,
        table: &'static str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = crate::api::PrefixResult> + '_>, StoreError> {
        self.ro_tx.prefix_iterator(table, prefix)
    }
}

impl StorageRwTx for FjallRwTx {
    fn put_batch(
        &mut self,
        table: &'static str,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        let mut partitions = std::collections::HashMap::new();
        for (key, value) in batch {
            let partition = partitions
                .entry(table)
                .or_insert_with(|| self.ro_tx.backend.get_partition(table).unwrap());
            self.write_batch.insert(partition, key, value);
        }
        Ok(())
    }

    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        let partition = self.ro_tx.backend.get_partition(table).unwrap();
        partition.remove(key).unwrap();
        Ok(())
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        let empty_batch = self.ro_tx.backend.keyspace.batch();
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
        Ok(self.snapshot.get(key).unwrap().map(|v| v.to_vec()))
    }
}
