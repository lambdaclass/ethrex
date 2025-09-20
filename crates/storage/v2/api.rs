use std::{fmt::Debug, sync::Arc};

use ethrex_common::H256;

use crate::error::StoreError;

pub type PrefixIterator<'a> = Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<u8>), StoreError>> + 'a>;

pub trait StorageBackend: Debug + Send + Sync {
    fn open(path: &str) -> Result<Arc<dyn StorageBackend>, StoreError>
    where
        Self: Sized;
    fn create_table(&self, name: &str, options: TableOptions) -> Result<(), StoreError>;
    fn clear_table(&self, table: &str) -> Result<(), StoreError>;
    fn begin_read<'a>(&'a self) -> Result<Box<dyn StorageRoTx<'a> + 'a>, StoreError>;
    fn begin_write<'a>(&'a self) -> Result<Box<dyn StorageRwTx<'a> + 'a>, StoreError>;
    fn begin_locked(
        &self,
        table_name: &str,
        address_prefix: Option<H256>,
    ) -> Result<Box<dyn StorageLocked>, StoreError>;
}

pub struct TableOptions {
    pub dupsort: bool,
}

pub trait StorageRoTx<'a> {
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError>;

    /// Returns iterator over all key-value pairs where key starts with prefix
    fn prefix_iterator(&self, table: &str, prefix: &[u8]) -> Result<PrefixIterator, StoreError>;
}

pub trait StorageRwTx<'a> {
    fn put(&self, table: &str, key: &[u8], value: &[u8]) -> Result<(), StoreError>;
    fn delete(&self, table: &str, key: &[u8]) -> Result<(), StoreError>;
    fn commit(self: Box<Self>) -> Result<(), StoreError>;
}

pub trait StorageLocked: Send + Sync {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError>;
}
