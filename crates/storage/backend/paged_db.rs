//! PagedDb backend implementation.
//!
//! This module provides a storage backend using ethrex_db's PagedDb,
//! a Paprika-inspired memory-mapped page-based storage engine.
//!
//! ## Design
//!
//! Since PagedDb is page-based rather than key-value based, we simulate
//! table-based key-value storage using:
//!
//! 1. **Table prefixing**: Each table gets a 1-byte prefix prepended to keys
//! 2. **In-memory index**: A BTreeMap tracks all key-value pairs per table
//! 3. **Page storage**: Data is serialized to pages on commit
//!
//! This approach allows us to:
//! - Support multiple "tables" in a single PagedDb
//! - Provide efficient prefix iteration
//! - Maintain compatibility with the StorageBackend trait

use crate::api::tables::TABLES;
use crate::api::{
    PrefixResult, StorageBackend, StorageLockedView, StorageReadView, StorageWriteBatch,
};
use crate::error::StoreError;
use ethrex_db::store::{CommitOptions, PageType, PagedDb, PAGE_SIZE};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, RwLock};
use tracing::debug;

/// Table prefix mapping - each table gets a unique 1-byte prefix.
fn table_prefix(table: &str) -> Option<u8> {
    TABLES.iter().position(|&t| t == table).map(|i| i as u8)
}

/// Prefixes a key with the table identifier.
fn prefix_key(table: &str, key: &[u8]) -> Option<Vec<u8>> {
    let prefix = table_prefix(table)?;
    let mut prefixed = Vec::with_capacity(1 + key.len());
    prefixed.push(prefix);
    prefixed.extend_from_slice(key);
    Some(prefixed)
}

/// Type alias for the in-memory data store.
type DataStore = BTreeMap<Vec<u8>, Vec<u8>>;

/// PagedDb-based storage backend.
///
/// Uses memory-mapped file storage with Copy-on-Write semantics.
/// Data is stored in an in-memory BTreeMap and persisted to pages on commit.
pub struct PagedDbBackend {
    /// The underlying PagedDb storage.
    db: Arc<RwLock<PagedDb>>,
    /// In-memory data store (persisted to pages).
    data: Arc<RwLock<DataStore>>,
    /// Path to the database file.
    path: std::path::PathBuf,
}

impl std::fmt::Debug for PagedDbBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PagedDbBackend")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl PagedDbBackend {
    /// Opens or creates a PagedDb at the given path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StoreError::Custom(format!("Failed to create directory: {e}")))?;
        }

        // Construct db file path
        let db_path = path.join("paged.db");

        let db = PagedDb::open(&db_path)
            .map_err(|e| StoreError::Custom(format!("Failed to open PagedDb: {e}")))?;

        debug!("Opened PagedDb at {:?}", db_path);

        // Load existing data from pages (if any)
        let data = Self::load_data_from_pages(&db)?;

        Ok(Self {
            db: Arc::new(RwLock::new(db)),
            data: Arc::new(RwLock::new(data)),
            path: path.to_path_buf(),
        })
    }

    /// Creates an in-memory PagedDb (for testing).
    pub fn in_memory() -> Result<Self, StoreError> {
        let db = PagedDb::in_memory(16384)
            .map_err(|e| StoreError::Custom(format!("Failed to create in-memory PagedDb: {e}")))?;

        Ok(Self {
            db: Arc::new(RwLock::new(db)),
            data: Arc::new(RwLock::new(BTreeMap::new())),
            path: std::path::PathBuf::new(),
        })
    }

    /// Loads data from existing pages.
    ///
    /// Data is stored in data pages with a simple format:
    /// - Each entry: [key_len: u16][value_len: u32][key][value]
    fn load_data_from_pages(db: &PagedDb) -> Result<DataStore, StoreError> {
        let mut data = BTreeMap::new();

        let read = db.begin_read_only();
        let state_root = read.state_root();

        // If no state root, database is empty
        if state_root.is_null() {
            return Ok(data);
        }

        // Read data from the state root page and subsequent pages
        // For simplicity in this initial implementation, we store all data
        // serialized in a single chain of pages starting from state_root.

        let mut current_addr = state_root;
        let mut buffer = Vec::new();

        while !current_addr.is_null() {
            let page = read
                .get_page(current_addr)
                .map_err(|e| StoreError::Custom(format!("Failed to read page: {e}")))?;

            let page_data = page.as_bytes();

            // Extract next page address from header (bytes 4-8)
            let next_addr_raw = u32::from_le_bytes([
                page_data[4],
                page_data[5],
                page_data[6],
                page_data[7],
            ]);
            current_addr = ethrex_db::store::DbAddress::from(next_addr_raw);

            // Data starts after 8-byte header
            let data_len = u16::from_le_bytes([page_data[8], page_data[9]]) as usize;
            if data_len > 0 && data_len <= PAGE_SIZE - 10 {
                buffer.extend_from_slice(&page_data[10..10 + data_len]);
            }
        }

        // Parse entries from buffer
        let mut pos = 0;
        while pos + 6 <= buffer.len() {
            let key_len = u16::from_le_bytes([buffer[pos], buffer[pos + 1]]) as usize;
            let value_len = u32::from_le_bytes([
                buffer[pos + 2],
                buffer[pos + 3],
                buffer[pos + 4],
                buffer[pos + 5],
            ]) as usize;

            pos += 6;
            if pos + key_len + value_len > buffer.len() {
                break;
            }

            let key = buffer[pos..pos + key_len].to_vec();
            let value = buffer[pos + key_len..pos + key_len + value_len].to_vec();
            data.insert(key, value);

            pos += key_len + value_len;
        }

        debug!("Loaded {} entries from PagedDb", data.len());
        Ok(data)
    }

    /// Persists the current data store to pages.
    fn persist_data(&self) -> Result<(), StoreError> {
        let data = self
            .data
            .read()
            .map_err(|_| StoreError::Custom("Failed to acquire read lock".to_string()))?;

        let mut db = self
            .db
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire write lock".to_string()))?;

        let mut batch = db.begin_batch();

        // Serialize all data to a buffer
        let mut buffer = Vec::new();
        for (key, value) in data.iter() {
            // Format: [key_len: u16][value_len: u32][key][value]
            buffer.extend_from_slice(&(key.len() as u16).to_le_bytes());
            buffer.extend_from_slice(&(value.len() as u32).to_le_bytes());
            buffer.extend_from_slice(key);
            buffer.extend_from_slice(value);
        }

        // Write buffer to pages
        let data_per_page = PAGE_SIZE - 10; // 8 byte header + 2 byte data length
        let mut chunks = buffer.chunks(data_per_page).peekable();
        let mut first_addr = None;
        let mut prev_page_addr = None;

        while let Some(chunk) = chunks.next() {
            let (addr, mut page) = batch
                .allocate_page(PageType::Data, 0)
                .map_err(|e| StoreError::Custom(format!("Failed to allocate page: {e}")))?;

            if first_addr.is_none() {
                first_addr = Some(addr);
            }

            // Update previous page to point to this one
            if let Some(prev_addr) = prev_page_addr {
                let mut prev_page = batch
                    .get_writable_copy(prev_addr)
                    .map_err(|e| StoreError::Custom(format!("Failed to get page: {e}")))?;

                let page_bytes = prev_page.as_bytes_mut();
                let addr_bytes = addr.raw().to_le_bytes();
                page_bytes[4..8].copy_from_slice(&addr_bytes);
                batch.mark_dirty(prev_addr, prev_page);
            }

            // Write data to page
            let page_bytes = page.as_bytes_mut();
            // Header bytes 4-7: next page address (0 for last page)
            page_bytes[4..8].copy_from_slice(&[0, 0, 0, 0]);
            // Bytes 8-9: data length
            page_bytes[8..10].copy_from_slice(&(chunk.len() as u16).to_le_bytes());
            // Data
            page_bytes[10..10 + chunk.len()].copy_from_slice(chunk);

            batch.mark_dirty(addr, page);
            prev_page_addr = Some(addr);
        }

        // Set state root to first data page
        if let Some(addr) = first_addr {
            batch.set_state_root(addr);
        }

        batch
            .commit(CommitOptions::FlushDataAndRoot)
            .map_err(|e| StoreError::Custom(format!("Failed to commit: {e}")))?;

        Ok(())
    }
}

impl StorageBackend for PagedDbBackend {
    fn clear_table(&self, table: &'static str) -> Result<(), StoreError> {
        let prefix = table_prefix(table)
            .ok_or_else(|| StoreError::Custom(format!("Unknown table: {table}")))?;

        let mut data = self
            .data
            .write()
            .map_err(|_| StoreError::Custom("Failed to acquire write lock".to_string()))?;

        // Remove all keys with this table prefix
        data.retain(|k, _| k.first() != Some(&prefix));

        Ok(())
    }

    fn begin_read(&self) -> Result<Box<dyn StorageReadView + '_>, StoreError> {
        Ok(Box::new(PagedDbReadView {
            data: &self.data,
        }))
    }

    fn begin_write(&self) -> Result<Box<dyn StorageWriteBatch + 'static>, StoreError> {
        Ok(Box::new(PagedDbWriteBatch {
            backend: PagedDbBackend {
                db: self.db.clone(),
                data: self.data.clone(),
                path: self.path.clone(),
            },
            pending: BTreeMap::new(),
            deletions: Vec::new(),
        }))
    }

    fn begin_locked(
        &self,
        table_name: &'static str,
    ) -> Result<Box<dyn StorageLockedView>, StoreError> {
        // Take a snapshot of the current data for this table
        let prefix = table_prefix(table_name)
            .ok_or_else(|| StoreError::Custom(format!("Unknown table: {table_name}")))?;

        let data = self
            .data
            .read()
            .map_err(|_| StoreError::Custom("Failed to acquire read lock".to_string()))?;

        // Clone relevant data for this table
        let snapshot: BTreeMap<Vec<u8>, Vec<u8>> = data
            .iter()
            .filter(|(k, _)| k.first() == Some(&prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        Ok(Box::new(PagedDbLockedView {
            prefix,
            data: snapshot,
        }))
    }

    fn create_checkpoint(&self, path: &Path) -> Result<(), StoreError> {
        // For PagedDb, we create a checkpoint by copying the database file
        // and the in-memory data

        // First, persist current data
        self.persist_data()?;

        // Then copy the database file
        let source_db = self.path.join("paged.db");
        let dest_db = path.join("paged.db");

        if source_db.exists() {
            std::fs::create_dir_all(path)
                .map_err(|e| StoreError::Custom(format!("Failed to create checkpoint dir: {e}")))?;

            std::fs::copy(&source_db, &dest_db)
                .map_err(|e| StoreError::Custom(format!("Failed to copy database: {e}")))?;
        }

        Ok(())
    }
}

/// Read-only view for PagedDb.
pub struct PagedDbReadView<'a> {
    data: &'a RwLock<DataStore>,
}

impl<'a> StorageReadView for PagedDbReadView<'a> {
    fn get(&self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let prefixed_key = prefix_key(table, key)
            .ok_or_else(|| StoreError::Custom(format!("Unknown table: {table}")))?;

        let data = self
            .data
            .read()
            .map_err(|_| StoreError::Custom("Failed to acquire read lock".to_string()))?;

        Ok(data.get(&prefixed_key).cloned())
    }

    fn prefix_iterator(
        &self,
        table: &'static str,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = PrefixResult> + '_>, StoreError> {
        let table_prefix_byte = table_prefix(table)
            .ok_or_else(|| StoreError::Custom(format!("Unknown table: {table}")))?;

        // Build the full prefix (table prefix + user prefix)
        let mut full_prefix = Vec::with_capacity(1 + prefix.len());
        full_prefix.push(table_prefix_byte);
        full_prefix.extend_from_slice(prefix);

        let data = self
            .data
            .read()
            .map_err(|_| StoreError::Custom("Failed to acquire read lock".to_string()))?;

        // Collect matching entries
        let results: Vec<PrefixResult> = data
            .range(full_prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&full_prefix))
            .map(|(k, v)| {
                // Remove table prefix from key before returning
                let key_without_prefix = k[1..].to_vec();
                Ok((key_without_prefix.into_boxed_slice(), v.clone().into_boxed_slice()))
            })
            .collect();

        Ok(Box::new(results.into_iter()))
    }
}

/// Write batch for PagedDb.
pub struct PagedDbWriteBatch {
    backend: PagedDbBackend,
    pending: BTreeMap<Vec<u8>, Vec<u8>>,
    deletions: Vec<Vec<u8>>,
}

impl StorageWriteBatch for PagedDbWriteBatch {
    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        let prefixed_key = prefix_key(table, key)
            .ok_or_else(|| StoreError::Custom(format!("Unknown table: {table}")))?;

        self.pending.insert(prefixed_key, value.to_vec());
        Ok(())
    }

    fn put_batch(
        &mut self,
        table: &'static str,
        batch: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<(), StoreError> {
        let table_prefix_byte = table_prefix(table)
            .ok_or_else(|| StoreError::Custom(format!("Unknown table: {table}")))?;

        for (key, value) in batch {
            let mut prefixed_key = Vec::with_capacity(1 + key.len());
            prefixed_key.push(table_prefix_byte);
            prefixed_key.extend_from_slice(&key);
            self.pending.insert(prefixed_key, value);
        }

        Ok(())
    }

    fn delete(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        let prefixed_key = prefix_key(table, key)
            .ok_or_else(|| StoreError::Custom(format!("Unknown table: {table}")))?;

        self.deletions.push(prefixed_key);
        Ok(())
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        // Apply pending changes to the data store
        {
            let mut data = self
                .backend
                .data
                .write()
                .map_err(|_| StoreError::Custom("Failed to acquire write lock".to_string()))?;

            // Apply insertions
            let pending = std::mem::take(&mut self.pending);
            for (key, value) in pending {
                data.insert(key, value);
            }

            // Apply deletions
            let deletions = std::mem::take(&mut self.deletions);
            for key in deletions {
                data.remove(&key);
            }
        }

        // Persist to PagedDb
        self.backend.persist_data()?;

        Ok(())
    }
}

/// Locked snapshot view for PagedDb.
pub struct PagedDbLockedView {
    prefix: u8,
    data: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl StorageLockedView for PagedDbLockedView {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let mut prefixed_key = Vec::with_capacity(1 + key.len());
        prefixed_key.push(self.prefix);
        prefixed_key.extend_from_slice(key);

        Ok(self.data.get(&prefixed_key).cloned())
    }
}

// Safety: PagedDbLockedView contains only owned data
unsafe impl Send for PagedDbLockedView {}
unsafe impl Sync for PagedDbLockedView {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tables::HEADERS;

    #[test]
    fn test_basic_operations() {
        let backend = PagedDbBackend::in_memory().expect("Failed to create backend");

        // Write data
        {
            let mut batch = backend.begin_write().expect("Failed to begin write");
            batch
                .put(HEADERS, b"key1", b"value1")
                .expect("Failed to put");
            batch
                .put(HEADERS, b"key2", b"value2")
                .expect("Failed to put");
            batch.commit().expect("Failed to commit");
        }

        // Read data
        {
            let read = backend.begin_read().expect("Failed to begin read");
            assert_eq!(
                read.get(HEADERS, b"key1").expect("Failed to get"),
                Some(b"value1".to_vec())
            );
            assert_eq!(
                read.get(HEADERS, b"key2").expect("Failed to get"),
                Some(b"value2".to_vec())
            );
            assert_eq!(
                read.get(HEADERS, b"key3").expect("Failed to get"),
                None
            );
        }
    }

    #[test]
    fn test_delete() {
        let backend = PagedDbBackend::in_memory().expect("Failed to create backend");

        // Write data
        {
            let mut batch = backend.begin_write().expect("Failed to begin write");
            batch
                .put(HEADERS, b"key1", b"value1")
                .expect("Failed to put");
            batch.commit().expect("Failed to commit");
        }

        // Delete data
        {
            let mut batch = backend.begin_write().expect("Failed to begin write");
            batch.delete(HEADERS, b"key1").expect("Failed to delete");
            batch.commit().expect("Failed to commit");
        }

        // Verify deletion
        {
            let read = backend.begin_read().expect("Failed to begin read");
            assert_eq!(
                read.get(HEADERS, b"key1").expect("Failed to get"),
                None
            );
        }
    }

    #[test]
    fn test_prefix_iterator() {
        let backend = PagedDbBackend::in_memory().expect("Failed to create backend");

        // Write data with common prefix
        {
            let mut batch = backend.begin_write().expect("Failed to begin write");
            batch
                .put(HEADERS, b"prefix_a", b"value_a")
                .expect("Failed to put");
            batch
                .put(HEADERS, b"prefix_b", b"value_b")
                .expect("Failed to put");
            batch
                .put(HEADERS, b"other", b"value_other")
                .expect("Failed to put");
            batch.commit().expect("Failed to commit");
        }

        // Iterate with prefix
        {
            let read = backend.begin_read().expect("Failed to begin read");
            let mut iter = read
                .prefix_iterator(HEADERS, b"prefix_")
                .expect("Failed to create iterator");

            let results: Vec<_> = iter.by_ref().collect();
            assert_eq!(results.len(), 2);
        }
    }

    #[test]
    fn test_clear_table() {
        let backend = PagedDbBackend::in_memory().expect("Failed to create backend");

        // Write data
        {
            let mut batch = backend.begin_write().expect("Failed to begin write");
            batch
                .put(HEADERS, b"key1", b"value1")
                .expect("Failed to put");
            batch.commit().expect("Failed to commit");
        }

        // Clear table
        backend.clear_table(HEADERS).expect("Failed to clear");

        // Verify cleared
        {
            let read = backend.begin_read().expect("Failed to begin read");
            assert_eq!(
                read.get(HEADERS, b"key1").expect("Failed to get"),
                None
            );
        }
    }
}
