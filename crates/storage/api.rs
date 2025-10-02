//! # Storage Backend API
//!
//! This module provides a thin, minimal interface for storage backends:
//!
//! - Thin: Minimal set of operations that databases must provide
//! - Simple: Avoids type-system complexity and focuses on core functionality
//!
//! Rather than implementing business logic in each database backend, this API
//! provides low-level primitives that higher-level code can build upon.
//! This eliminates code duplication and makes adding new database backends trivial.
//!
//! The API differentiates between three types of database access:
//!
//! - Read-only transactions ([`StorageRoTx`])
//! - Read-write transactions ([`StorageRwTx`])
//! - Locked snapshots ([`StorageLocked`]): Persistent read-only views, righ now it's
//!   only used in snapsync stage.

use crate::error::StoreError;
use std::{fmt::Debug, path::Path};

pub use tables::LIST as TABLES;

/// Type alias for the result of a prefix iterator.
pub type PrefixResult = Result<(Vec<u8>, Vec<u8>), StoreError>;

/// Configuration options for table creation.
#[derive(Debug, Clone)]
pub struct TableOptions {
    /// Whether the table supports duplicate keys, this means that multiple values can be stored for the same key.
    /// This is useful for certain indexing scenarios but not supported by all backends.
    pub dupsort: bool,
}

/// This trait provides a minimal set of operations required from a database backend.
/// Implementations should focus on providing efficient access to the underlying storage
/// without implementing business logic.
pub trait StorageBackend: Debug + Send + Sync {
    /// Opens a storage backend at the specified path.
    fn open(path: impl AsRef<Path>) -> Result<Self, StoreError>
    where
        Self: Sized;

    /// Creates a new table, allowing to specify [`TableOptions`].
    fn create_table(&self, name: &'static str, options: TableOptions) -> Result<(), StoreError>;

    /// Removes all data from the specified table.
    fn clear_table(&self, table: &'static str) -> Result<(), StoreError>;

    /// Begins a new read-only transaction.
    fn begin_read(&self) -> Result<Box<dyn StorageRoTx + '_>, StoreError>;

    /// Begins a new read-write transaction.
    fn begin_write(&self) -> Result<Box<dyn StorageRwTx + '_>, StoreError>;

    /// Creates a locked snapshot for a specific table.
    ///
    /// This provides a persistent read-only view of a single table, optimized
    /// for batch read operations. The snapshot remains valid until dropped.
    fn begin_locked(&self, table_name: &'static str) -> Result<Box<dyn StorageLocked>, StoreError>;
}

/// Read-only transaction interface.
/// Provides methods to read data from the database
pub trait StorageRoTx {
    /// Retrieves a value by key from the specified table.
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError>;

    /// Returns an iterator over all key-value pairs with the given prefix.
    fn prefix_iterator(
        &self,
        table: &'static str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError>;
}

/// Read-write transaction interface.
///
/// Extends [`StorageRoTx`] with methods to modify the database.
/// Changes are not persisted until [`commit()`](StorageRwTx::commit) is called.
pub trait StorageRwTx: StorageRoTx {
    /// Stores a key-value pair in the specified table.
    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        self.put_batch(vec![(table, key.to_vec(), value.to_vec())])
    }

    /// Stores multiple key-value pairs in the specified table within the transaction.
    fn put_batch(&mut self, batch: Vec<(&'static str, Vec<u8>, Vec<u8>)>)
    -> Result<(), StoreError>;

    /// Removes a key-value pair from the specified table.
    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError>;

    /// Commits all changes made in this transaction.
    fn commit(self: Box<Self>) -> Result<(), StoreError>;
}

/// Locked snapshot interface for batch read operations.
/// Provides read-only access to a specific table with a persistent snapshot.
/// This is optimized for scenarios where many reads are performed on the same
/// table, such as trie traversal operations.
/// This is currently only used in snapsync stage.
/// TODO: Check if we can remove this trait and use [`StorageRoTx`] instead.
pub trait StorageLocked: Send + Sync + 'static {
    /// Retrieves a value by key from the locked table.
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError>;
}

/// Table names used by the storage engine.
pub mod tables {
    pub const CHAIN_DATA: &str = "chain_data";
    pub const ACCOUNT_CODES: &str = "account_codes";
    pub const BODIES: &str = "bodies";
    pub const BLOCK_NUMBERS: &str = "block_numbers";
    pub const CANONICAL_BLOCK_HASHES: &str = "canonical_block_hashes";
    pub const HEADERS: &str = "headers";
    pub const PENDING_BLOCKS: &str = "pending_blocks";
    pub const TRANSACTION_LOCATIONS: &str = "transaction_locations";
    pub const RECEIPTS: &str = "receipts";
    pub const SNAP_STATE: &str = "snap_state";
    pub const INVALID_CHAINS: &str = "invalid_chains";
    pub const STATE_TRIE_NODES: &str = "state_trie_nodes";
    pub const STORAGE_TRIE_NODES: &str = "storage_trie_nodes";

    pub const LIST: [&str; 13] = [
        CHAIN_DATA,
        ACCOUNT_CODES,
        BODIES,
        BLOCK_NUMBERS,
        CANONICAL_BLOCK_HASHES,
        HEADERS,
        PENDING_BLOCKS,
        TRANSACTION_LOCATIONS,
        RECEIPTS,
        SNAP_STATE,
        INVALID_CHAINS,
        STATE_TRIE_NODES,
        STORAGE_TRIE_NODES,
    ];
}
