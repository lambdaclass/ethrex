use std::{fmt::Debug, path::Path, sync::Arc};

use crate::error::StoreError;

pub trait StorageBackend: Debug + Send + Sync + 'static {
    type ReadTx<'a>: StorageRoTx
    where
        Self: 'a;
    type WriteTx<'a>: StorageRwTx
    where
        Self: 'a;
    type Locked<'a>: StorageLocked
    where
        Self: 'a;

    fn open(path: impl AsRef<Path>) -> Result<Arc<Self>, StoreError>
    where
        Self: Sized;
    fn create_table(&self, name: &str, options: TableOptions) -> Result<(), StoreError>;
    fn clear_table(&self, table: &str) -> Result<(), StoreError>;
    fn begin_read(&self) -> Result<Self::ReadTx<'_>, StoreError>;
    fn begin_write(&self) -> Result<Self::WriteTx<'_>, StoreError>;
    fn begin_locked(&self, table_name: &str) -> Result<Self::Locked<'_>, StoreError>;
}

pub struct TableOptions {
    pub dupsort: bool,
}

pub trait StorageRoTx {
    type PrefixIter: Iterator<Item = Result<(Vec<u8>, Vec<u8>), StoreError>>;

    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError>;

    /// Returns iterator over all key-value pairs where key starts with prefix
    fn prefix_iterator(&self, table: &str, prefix: &[u8]) -> Result<Self::PrefixIter, StoreError>;
}

pub trait StorageRwTx: StorageRoTx {
    fn put(&self, table: &str, key: &[u8], value: &[u8]) -> Result<(), StoreError>;
    fn delete(&self, table: &str, key: &[u8]) -> Result<(), StoreError>;
    fn commit(self) -> Result<(), StoreError>;
}

pub trait StorageLocked: Send + Sync {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError>;
}
