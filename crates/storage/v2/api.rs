use std::sync::Arc;

use crate::error::StoreError;

pub trait StorageBackend: Send + Sync {
    fn open(path: &str) -> Result<Arc<impl StorageBackend>, StoreError>
    where
        Self: Sized;
    fn create_table(&self, name: &str, options: TableOptions) -> Result<(), StoreError>;
    fn begin_read<'a>(&'a self) -> Result<Box<dyn StorageRoTx<'a> + 'a>, StoreError>;
    fn begin_write<'a>(&'a self) -> Result<Box<dyn StorageRwTx<'a> + 'a>, StoreError>;
}

pub struct TableOptions {
    pub dupsort: bool,
}

pub trait StorageRoTx<'a> {
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError>;
    // Should add a cursor method
}

pub trait StorageRwTx<'a> {
    fn put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), StoreError>;
    fn delete(&mut self, table: &str, key: &[u8]) -> Result<(), StoreError>;
    fn commit(self: Box<Self>) -> Result<(), StoreError>;
}
