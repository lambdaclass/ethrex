use std::{fmt::Debug, path::Path, sync::Arc};

use crate::error::StoreError;

pub type PrefixResult = Result<(Vec<u8>, Vec<u8>), StoreError>;

pub const TABLES: [&str; 13] = [
    "chain_data",
    "account_codes",
    "bodies",
    "block_numbers",
    "canonical_block_hashes",
    "headers",
    "pending_blocks",
    "transaction_locations",
    "receipts",
    "snap_state",
    "invalid_chains",
    "state_trie_nodes",
    "storage_trie_nodes",
];

pub trait StorageBackend: Debug + Send + Sync + 'static {
    fn open(path: impl AsRef<Path>) -> Result<Arc<Self>, StoreError>
    where
        Self: Sized;
    fn create_table(&self, name: &str, options: TableOptions) -> Result<(), StoreError>;
    fn clear_table(&self, table: &str) -> Result<(), StoreError>;
    fn begin_read(&self) -> Result<Box<dyn StorageRoTx + '_>, StoreError>;
    fn begin_write(&self) -> Result<Box<dyn StorageRwTx + '_>, StoreError>;
    fn begin_locked(&self, table_name: &str) -> Result<Box<dyn StorageLocked>, StoreError>;
}

pub struct TableOptions {
    pub dupsort: bool,
}

pub trait StorageRoTx {
    fn get(&self, table: &str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError>;
    fn prefix_iterator(
        &self,
        table: &str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError>;
}

pub trait StorageRwTx: StorageRoTx {
    fn put(&self, table: &str, key: &[u8], value: &[u8]) -> Result<(), StoreError>;
    fn delete(&self, table: &str, key: &[u8]) -> Result<(), StoreError>;
    fn commit(self: Box<Self>) -> Result<(), StoreError>;
}

pub trait StorageLocked: Send + Sync + 'static {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError>;
}
