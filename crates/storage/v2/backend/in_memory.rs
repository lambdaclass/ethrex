use crate::error::StoreError;
use crate::v2::api::{PrefixIterator, StorageBackend, StorageRoTx, StorageRwTx, TableOptions};
use std::sync::Arc;

#[derive(Debug)]
pub struct InMemoryBackend;

impl StorageBackend for InMemoryBackend {
    fn open(_path: &str) -> Result<Arc<dyn StorageBackend>, StoreError>
    where
        Self: Sized,
    {
        todo!()
    }

    fn create_table(&self, _name: &str, _options: TableOptions) -> Result<(), StoreError> {
        todo!()
    }

    fn clear_table(&self, _table: &str) -> Result<(), StoreError> {
        todo!()
    }

    fn begin_read<'a>(&'a self) -> Result<Box<dyn StorageRoTx<'a> + 'a>, StoreError> {
        todo!()
    }

    fn begin_write<'a>(&'a self) -> Result<Box<dyn StorageRwTx<'a> + 'a>, StoreError> {
        todo!()
    }
}

pub struct InMemoryRoTx;

impl StorageRoTx<'_> for InMemoryRoTx {
    fn get(&self, _table: &str, _key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        todo!()
    }

    fn prefix_iterator(&self, _table: &str, _prefix: &[u8]) -> Result<PrefixIterator, StoreError> {
        todo!()
    }
}

pub struct InMemoryRwTx;

impl StorageRwTx<'_> for InMemoryRwTx {
    fn put(&mut self, _table: &str, _key: &[u8], _value: &[u8]) -> Result<(), StoreError> {
        todo!()
    }

    fn delete(&mut self, _table: &str, _key: &[u8]) -> Result<(), StoreError> {
        todo!()
    }

    fn commit(self: Box<Self>) -> Result<(), StoreError> {
        todo!()
    }
}
