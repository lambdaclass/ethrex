//! PagedDb - Memory-mapped database with Copy-on-Write semantics.
//!
//! This implements a persistent storage engine using memory-mapped files,
//! inspired by LMDB and Paprika.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::RwLock;

use memmap2::MmapMut;
use parking_lot::Mutex;
use thiserror::Error;

use super::{DbAddress, Page, PageHeader, PageType, RootPage, PAGE_SIZE};

/// Database errors.
#[derive(Error, Debug)]
pub enum DbError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Database is corrupted")]
    Corrupted,
    #[error("Page not found: {0}")]
    PageNotFound(DbAddress),
    #[error("Database is full")]
    Full,
    #[error("Invalid page type")]
    InvalidPageType,
}

/// Result type for database operations.
pub type Result<T> = std::result::Result<T, DbError>;

/// Options for committing a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitOptions {
    /// Flush data pages but keep root in memory (faster, less durable).
    FlushDataOnly,
    /// Flush both data and root (slower, fully durable).
    FlushDataAndRoot,
    /// Don't flush (fastest, not durable until next flush).
    DangerNoFlush,
}

/// The main database structure.
///
/// Provides memory-mapped storage with Copy-on-Write semantics for
/// concurrent readers and a single writer.
pub struct PagedDb {
    /// Memory-mapped file (wrapped in Mutex for interior mutability).
    mmap: Mutex<MmapMut>,
    /// The underlying file.
    _file: Option<File>,
    /// Current batch ID (monotonically increasing).
    batch_id: u32,
    /// Root page (page 0).
    root: RwLock<RootPage>,
    /// Maximum number of pages.
    max_pages: u32,
}

impl PagedDb {
    /// Default initial size (64MB = 16384 pages).
    const DEFAULT_INITIAL_PAGES: u32 = 16384;

    /// Opens or creates a database at the given path.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_size(path, Self::DEFAULT_INITIAL_PAGES)
    }

    /// Opens or creates a database with the specified initial size.
    pub fn open_with_size<P: AsRef<Path>>(path: P, initial_pages: u32) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;

        let file_size = file.metadata()?.len();
        let min_size = (initial_pages as u64) * (PAGE_SIZE as u64);

        // Extend file if needed
        if file_size < min_size {
            file.set_len(min_size)?;
        }

        let actual_size = file.metadata()?.len();
        let max_pages = (actual_size / PAGE_SIZE as u64) as u32;

        // Safety: We have exclusive write access via Mutex
        let mmap = unsafe { MmapMut::map_mut(&file)? };

        // Check if this is a new database
        let is_new = file_size == 0 || mmap[0..PAGE_SIZE].iter().all(|&b| b == 0);

        let (root, batch_id) = if is_new {
            // Initialize new database
            let root = RootPage::new(1);
            (root, 1)
        } else {
            // Load existing root
            let mut page_data = [0u8; PAGE_SIZE];
            page_data.copy_from_slice(&mmap[0..PAGE_SIZE]);
            let page = Page::from_bytes(page_data);
            let root = RootPage::wrap(page);
            let batch_id = root.page().header().batch_id;
            (root, batch_id)
        };

        let mut db = Self {
            mmap: Mutex::new(mmap),
            _file: Some(file),
            batch_id,
            root: RwLock::new(root),
            max_pages,
        };

        // Write initial root if new
        if is_new {
            db.write_root()?;
        }

        Ok(db)
    }

    /// Creates an in-memory database (for testing).
    pub fn in_memory(pages: u32) -> Result<Self> {
        let size = (pages as usize) * PAGE_SIZE;
        let mmap = MmapMut::map_anon(size)?;

        let root = RootPage::new(1);

        let mut db = Self {
            mmap: Mutex::new(mmap),
            _file: None,
            batch_id: 1,
            root: RwLock::new(root),
            max_pages: pages,
        };

        db.write_root()?;
        Ok(db)
    }

    /// Writes the root page to the memory map.
    fn write_root(&mut self) -> Result<()> {
        let root = self.root.read().unwrap();
        let mut mmap = self.mmap.lock();
        mmap[0..PAGE_SIZE].copy_from_slice(root.page().as_bytes());
        Ok(())
    }

    /// Writes the root page (internal, for commit).
    fn write_root_internal(&self) -> Result<()> {
        let root = self.root.read().unwrap();
        let mut mmap = self.mmap.lock();
        mmap[0..PAGE_SIZE].copy_from_slice(root.page().as_bytes());
        Ok(())
    }

    /// Begins a new read-only batch.
    pub fn begin_read_only(&self) -> ReadOnlyBatch<'_> {
        let root = self.root.read().unwrap();
        ReadOnlyBatch {
            db: self,
            batch_id: root.page().header().batch_id,
            block_number: root.block_number(),
        }
    }

    /// Begins a new writable batch.
    pub fn begin_batch(&mut self) -> BatchContext<'_> {
        self.batch_id += 1;
        let batch_id = self.batch_id;

        BatchContext {
            db: self,
            batch_id,
            dirty_pages: HashMap::new(),
            allocated_pages: Vec::new(),
        }
    }

    /// Returns the current batch ID.
    pub fn batch_id(&self) -> u32 {
        self.batch_id
    }

    /// Returns the block number from the root.
    pub fn block_number(&self) -> u32 {
        self.root.read().unwrap().block_number()
    }

    /// Returns the block hash from the root.
    pub fn block_hash(&self) -> [u8; 32] {
        self.root.read().unwrap().block_hash()
    }

    /// Gets a page by address (read-only).
    pub fn get_page(&self, addr: DbAddress) -> Result<Page> {
        if addr.is_null() {
            return Err(DbError::PageNotFound(addr));
        }

        let offset = addr.file_offset() as usize;
        let mmap = self.mmap.lock();
        if offset + PAGE_SIZE > mmap.len() {
            return Err(DbError::PageNotFound(addr));
        }

        let mut page_data = [0u8; PAGE_SIZE];
        page_data.copy_from_slice(&mmap[offset..offset + PAGE_SIZE]);
        Ok(Page::from_bytes(page_data))
    }

    /// Flushes all changes to disk.
    pub fn flush(&self) -> Result<()> {
        let mmap = self.mmap.lock();
        mmap.flush()?;
        Ok(())
    }
}

/// A read-only view of the database.
pub struct ReadOnlyBatch<'a> {
    db: &'a PagedDb,
    batch_id: u32,
    block_number: u32,
}

impl<'a> ReadOnlyBatch<'a> {
    /// Returns the batch ID.
    pub fn batch_id(&self) -> u32 {
        self.batch_id
    }

    /// Returns the block number.
    pub fn block_number(&self) -> u32 {
        self.block_number
    }

    /// Gets a page by address.
    pub fn get_page(&self, addr: DbAddress) -> Result<Page> {
        self.db.get_page(addr)
    }

    /// Gets the state root address.
    pub fn state_root(&self) -> DbAddress {
        self.db.root.read().unwrap().state_root()
    }
}

/// A writable batch context.
///
/// Implements Copy-on-Write: pages are copied before modification.
pub struct BatchContext<'a> {
    db: &'a mut PagedDb,
    batch_id: u32,
    /// Pages modified in this batch (addr -> page data).
    dirty_pages: HashMap<DbAddress, Page>,
    /// Newly allocated pages in this batch.
    allocated_pages: Vec<DbAddress>,
}

impl<'a> BatchContext<'a> {
    /// Returns the batch ID.
    pub fn batch_id(&self) -> u32 {
        self.batch_id
    }

    /// Allocates a new page.
    pub fn allocate_page(&mut self, page_type: PageType, level: u8) -> Result<(DbAddress, Page)> {
        let addr = {
            let mut root = self.db.root.write().unwrap();
            let addr = root.allocate_page();
            if addr.raw() >= self.db.max_pages {
                return Err(DbError::Full);
            }
            addr
        };

        let mut page = Page::new();
        page.set_header(PageHeader::new(self.batch_id, page_type, level));

        self.dirty_pages.insert(addr, page.clone());
        self.allocated_pages.push(addr);

        Ok((addr, page))
    }

    /// Gets a page, returning a copy if it needs to be modified.
    pub fn get_page(&self, addr: DbAddress) -> Result<Page> {
        // Check dirty pages first
        if let Some(page) = self.dirty_pages.get(&addr) {
            return Ok(page.clone());
        }

        // Read from database
        self.db.get_page(addr)
    }

    /// Gets a writable copy of a page (Copy-on-Write).
    pub fn get_writable_copy(&mut self, addr: DbAddress) -> Result<Page> {
        // Already dirty?
        if let Some(page) = self.dirty_pages.get(&addr) {
            return Ok(page.clone());
        }

        // Read and copy
        let mut page = self.db.get_page(addr)?;
        let mut header = page.header();
        header.batch_id = self.batch_id;
        page.set_header(header);

        self.dirty_pages.insert(addr, page.clone());
        Ok(page)
    }

    /// Marks a page as dirty.
    pub fn mark_dirty(&mut self, addr: DbAddress, page: Page) {
        self.dirty_pages.insert(addr, page);
    }

    /// Sets metadata (block number and hash).
    pub fn set_metadata(&mut self, block_number: u32, block_hash: &[u8; 32]) {
        let mut root = self.db.root.write().unwrap();
        root.set_block_number(block_number);
        root.set_block_hash(block_hash);
    }

    /// Sets the state root address.
    pub fn set_state_root(&mut self, addr: DbAddress) {
        let mut root = self.db.root.write().unwrap();
        root.set_state_root(addr);
    }

    /// Commits the batch to the database.
    pub fn commit(self, options: CommitOptions) -> Result<()> {
        // Write dirty pages to mmap
        {
            let mut mmap = self.db.mmap.lock();
            for (addr, page) in &self.dirty_pages {
                let offset = addr.file_offset() as usize;
                if offset + PAGE_SIZE <= mmap.len() {
                    mmap[offset..offset + PAGE_SIZE].copy_from_slice(page.as_bytes());
                }
            }
        }

        // Update root batch ID
        {
            let mut root = self.db.root.write().unwrap();
            let mut header = root.page().header();
            header.batch_id = self.batch_id;
            root.page_mut().set_header(header);
        }

        // Write root page
        self.db.write_root_internal()?;

        // Flush based on options
        match options {
            CommitOptions::FlushDataAndRoot => {
                let mmap = self.db.mmap.lock();
                mmap.flush()?;
            }
            CommitOptions::FlushDataOnly => {
                let mmap = self.db.mmap.lock();
                mmap.flush_async()?;
            }
            CommitOptions::DangerNoFlush => {
                // Don't flush
            }
        }

        Ok(())
    }

    /// Aborts the batch, discarding all changes.
    pub fn abort(self) {
        // Just drop, changes are discarded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");

        let db = PagedDb::open(&path).unwrap();
        assert_eq!(db.batch_id(), 1);
        assert_eq!(db.block_number(), 0);
    }

    #[test]
    fn test_in_memory() {
        let db = PagedDb::in_memory(100).unwrap();
        assert_eq!(db.batch_id(), 1);
    }

    #[test]
    fn test_allocate_and_commit() {
        let mut db = PagedDb::in_memory(100).unwrap();

        let mut batch = db.begin_batch();
        let (addr, _page) = batch.allocate_page(PageType::Data, 0).unwrap();
        batch.set_metadata(42, &[0u8; 32]);
        batch.commit(CommitOptions::DangerNoFlush).unwrap();

        assert_eq!(db.block_number(), 42);

        // Verify page exists
        let page = db.get_page(addr).unwrap();
        assert_eq!(page.header().get_page_type(), Some(PageType::Data));
    }

    #[test]
    fn test_read_only_batch() {
        let db = PagedDb::in_memory(100).unwrap();
        let batch = db.begin_read_only();
        assert_eq!(batch.batch_id(), 1);
    }

    #[test]
    fn test_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("persist.db");

        // Create and write
        {
            let mut db = PagedDb::open(&path).unwrap();
            let mut batch = db.begin_batch();
            batch.set_metadata(100, &[1u8; 32]);
            batch.commit(CommitOptions::FlushDataAndRoot).unwrap();
        }

        // Reopen and verify
        {
            let db = PagedDb::open(&path).unwrap();
            assert_eq!(db.block_number(), 100);
            assert_eq!(db.block_hash(), [1u8; 32]);
        }
    }
}
