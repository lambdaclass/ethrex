//! PagedDb - Memory-mapped database with Copy-on-Write semantics.
//!
//! This implements a persistent storage engine using memory-mapped files,
//! inspired by LMDB and Paprika.
//!
//! ## Lock-Free Readers
//!
//! The database supports efficient concurrent readers through:
//! - Atomic metadata for frequently-accessed root state
//! - Direct mmap reads without locking for page access
//! - parking_lot::RwLock for minimal lock contention

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use memmap2::MmapMut;
use parking_lot::{Mutex, RwLock};
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
///
/// ## Performance Notes
///
/// Frequently-accessed metadata is stored atomically to allow lock-free reads:
/// - `batch_id`: Current batch ID
/// - `block_number`: Current block number
/// - `state_root_raw`: State root address (as raw u32)
///
/// The full root page is still protected by RwLock for writes,
/// but readers can access common metadata without locking.
pub struct PagedDb {
    /// Memory-mapped file (wrapped in Mutex for interior mutability).
    mmap: Mutex<MmapMut>,
    /// The underlying file.
    _file: Option<File>,
    /// Current batch ID (atomic for lock-free reads).
    batch_id: AtomicU32,
    /// Block number (atomic for lock-free reads).
    block_number_atomic: AtomicU32,
    /// State root address (atomic for lock-free reads).
    state_root_atomic: AtomicU32,
    /// Block hash as two u64s (atomic for lock-free reads).
    block_hash_low: AtomicU64,
    block_hash_high: AtomicU64,
    block_hash_mid1: AtomicU64,
    block_hash_mid2: AtomicU64,
    /// Root page (page 0) - protected by RwLock for full access.
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

        // Extract atomic values from root
        let block_number = root.block_number();
        let block_hash = root.block_hash();
        let state_root = root.state_root();

        let mut db = Self {
            mmap: Mutex::new(mmap),
            _file: Some(file),
            batch_id: AtomicU32::new(batch_id),
            block_number_atomic: AtomicU32::new(block_number),
            state_root_atomic: AtomicU32::new(state_root.raw()),
            block_hash_low: AtomicU64::new(u64::from_le_bytes(block_hash[0..8].try_into().unwrap())),
            block_hash_mid1: AtomicU64::new(u64::from_le_bytes(block_hash[8..16].try_into().unwrap())),
            block_hash_mid2: AtomicU64::new(u64::from_le_bytes(block_hash[16..24].try_into().unwrap())),
            block_hash_high: AtomicU64::new(u64::from_le_bytes(block_hash[24..32].try_into().unwrap())),
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
            batch_id: AtomicU32::new(1),
            block_number_atomic: AtomicU32::new(0),
            state_root_atomic: AtomicU32::new(DbAddress::NULL.raw()),
            block_hash_low: AtomicU64::new(0),
            block_hash_mid1: AtomicU64::new(0),
            block_hash_mid2: AtomicU64::new(0),
            block_hash_high: AtomicU64::new(0),
            root: RwLock::new(root),
            max_pages: pages,
        };

        db.write_root()?;
        Ok(db)
    }

    /// Writes the root page to the memory map.
    fn write_root(&mut self) -> Result<()> {
        let root = self.root.read();
        let mut mmap = self.mmap.lock();
        mmap[0..PAGE_SIZE].copy_from_slice(root.page().as_bytes());
        Ok(())
    }

    /// Writes the root page (internal, for commit).
    fn write_root_internal(&self) -> Result<()> {
        let root = self.root.read();
        let mut mmap = self.mmap.lock();
        mmap[0..PAGE_SIZE].copy_from_slice(root.page().as_bytes());
        Ok(())
    }

    /// Begins a new read-only batch (lock-free for common metadata).
    pub fn begin_read_only(&self) -> ReadOnlyBatch<'_> {
        // Use atomics for lock-free read of common metadata
        ReadOnlyBatch {
            db: self,
            batch_id: self.batch_id.load(Ordering::Acquire),
            block_number: self.block_number_atomic.load(Ordering::Acquire),
        }
    }

    /// Begins a new writable batch.
    pub fn begin_batch(&mut self) -> BatchContext<'_> {
        let batch_id = self.batch_id.fetch_add(1, Ordering::SeqCst) + 1;

        BatchContext {
            db: self,
            batch_id,
            dirty_pages: HashMap::new(),
            allocated_pages: Vec::new(),
            abandoned_pages: Vec::new(),
        }
    }

    /// Returns the current batch ID (lock-free).
    #[inline]
    pub fn batch_id(&self) -> u32 {
        self.batch_id.load(Ordering::Acquire)
    }

    /// Returns the block number from the root (lock-free).
    #[inline]
    pub fn block_number(&self) -> u32 {
        self.block_number_atomic.load(Ordering::Acquire)
    }

    /// Returns the block hash from the root (lock-free).
    pub fn block_hash(&self) -> [u8; 32] {
        let mut hash = [0u8; 32];
        hash[0..8].copy_from_slice(&self.block_hash_low.load(Ordering::Acquire).to_le_bytes());
        hash[8..16].copy_from_slice(&self.block_hash_mid1.load(Ordering::Acquire).to_le_bytes());
        hash[16..24].copy_from_slice(&self.block_hash_mid2.load(Ordering::Acquire).to_le_bytes());
        hash[24..32].copy_from_slice(&self.block_hash_high.load(Ordering::Acquire).to_le_bytes());
        hash
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

    /// Gets the state root address (lock-free).
    pub fn state_root(&self) -> DbAddress {
        DbAddress::from(self.db.state_root_atomic.load(Ordering::Acquire))
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
    /// Pages abandoned in this batch (to be tracked for reuse).
    abandoned_pages: Vec<DbAddress>,
}

impl<'a> BatchContext<'a> {
    /// Returns the batch ID.
    pub fn batch_id(&self) -> u32 {
        self.batch_id
    }

    /// Allocates a new page, reusing abandoned pages when available.
    pub fn allocate_page(&mut self, page_type: PageType, level: u8) -> Result<(DbAddress, Page)> {
        let addr = {
            let mut root = self.db.root.write();
            let current_batch = self.batch_id;

            // Try to reuse an abandoned page first (fast path: inline storage)
            if let Some(addr) = root.pop_abandoned_inline(current_batch) {
                addr
            } else {
                // Check linked list of abandoned pages
                let abandoned_head = root.abandoned_head();
                if !abandoned_head.is_null() {
                    // Try to pop from the abandoned page list
                    if let Some(addr) = self.try_pop_from_abandoned_list(&mut root, abandoned_head) {
                        addr
                    } else {
                        // Allocate new page
                        let addr = root.allocate_page();
                        if addr.raw() >= self.db.max_pages {
                            return Err(DbError::Full);
                        }
                        addr
                    }
                } else {
                    // Allocate new page
                    let addr = root.allocate_page();
                    if addr.raw() >= self.db.max_pages {
                        return Err(DbError::Full);
                    }
                    addr
                }
            }
        };

        let mut page = Page::new();
        page.set_header(PageHeader::new(self.batch_id, page_type, level));

        self.dirty_pages.insert(addr, page.clone());
        self.allocated_pages.push(addr);

        Ok((addr, page))
    }

    /// Tries to pop a page address from the abandoned page linked list.
    fn try_pop_from_abandoned_list(&self, root: &mut RootPage, head_addr: DbAddress) -> Option<DbAddress> {
        use super::AbandonedPage;

        let reorg_depth = root.reorg_depth();
        let current_batch = self.batch_id;

        // Load the abandoned page
        let page = self.db.get_page(head_addr).ok()?;
        if page.header().get_page_type() != Some(PageType::Abandoned) {
            return None;
        }

        let mut abandoned = AbandonedPage::wrap(page);

        // Check if enough batches have passed
        let abandoned_at = abandoned.batch_abandoned();
        if current_batch < abandoned_at + reorg_depth {
            // Not old enough to reuse
            return None;
        }

        // Pop an address from this abandoned page
        if let Some(addr) = abandoned.pop() {
            // If the abandoned page is now empty, move to the next in the list
            if abandoned.count() == 0 {
                let next = abandoned.next();
                root.set_abandoned_head(next);
                // The empty abandoned page itself can be reused
                return Some(head_addr);
            }
            // Otherwise, we need to mark this abandoned page as dirty
            // (handled by caller via commit)
            return Some(addr);
        }

        None
    }

    /// Marks a page as abandoned for future reuse.
    /// Call this when doing Copy-on-Write to track the old page.
    pub fn abandon_page(&mut self, addr: DbAddress) {
        if !addr.is_null() {
            self.abandoned_pages.push(addr);
        }
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
    ///
    /// When a page from a previous batch is copied, the original is marked
    /// as abandoned for future reuse (after reorg depth passes).
    pub fn get_writable_copy(&mut self, addr: DbAddress) -> Result<Page> {
        // Already dirty in this batch?
        if let Some(page) = self.dirty_pages.get(&addr) {
            return Ok(page.clone());
        }

        // Read the original page
        let mut page = self.db.get_page(addr)?;
        let original_batch = page.header().batch_id;

        // Update header for this batch
        let mut header = page.header();
        header.batch_id = self.batch_id;
        page.set_header(header);

        self.dirty_pages.insert(addr, page.clone());

        // Mark original page as abandoned if it's from a previous batch
        // (pages from the current batch are new allocations, not COW)
        if original_batch < self.batch_id {
            self.abandoned_pages.push(addr);
        }

        Ok(page)
    }

    /// Marks a page as dirty.
    pub fn mark_dirty(&mut self, addr: DbAddress, page: Page) {
        self.dirty_pages.insert(addr, page);
    }

    /// Sets metadata (block number and hash).
    pub fn set_metadata(&mut self, block_number: u32, block_hash: &[u8; 32]) {
        let mut root = self.db.root.write();
        root.set_block_number(block_number);
        root.set_block_hash(block_hash);
    }

    /// Sets the state root address.
    pub fn set_state_root(&mut self, addr: DbAddress) {
        let mut root = self.db.root.write();
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

        // Track abandoned pages from this batch
        if !self.abandoned_pages.is_empty() {
            use super::AbandonedPage;

            let mut root = self.db.root.write();
            let mut overflow_pages: Vec<DbAddress> = Vec::new();

            for &addr in &self.abandoned_pages {
                // Try inline storage first (fast path)
                if !root.try_add_abandoned_inline(addr, self.batch_id) {
                    // Inline storage full or batch mismatch - collect for overflow
                    overflow_pages.push(addr);
                }
            }

            // Handle overflow: create AbandonedPage nodes
            if !overflow_pages.is_empty() {
                let mut current_abandoned: Option<AbandonedPage> = None;
                let mut current_abandoned_addr: Option<DbAddress> = None;

                for addr in overflow_pages {
                    // Try to add to current abandoned page
                    let added = if let Some(ref mut ap) = current_abandoned {
                        ap.try_add(addr)
                    } else {
                        false
                    };

                    if !added {
                        // Save current abandoned page if exists
                        if let (Some(ap), Some(ap_addr)) = (current_abandoned.take(), current_abandoned_addr.take()) {
                            // Write the abandoned page to mmap
                            let mut mmap = self.db.mmap.lock();
                            let offset = ap_addr.file_offset() as usize;
                            if offset + PAGE_SIZE <= mmap.len() {
                                mmap[offset..offset + PAGE_SIZE].copy_from_slice(ap.into_page().as_bytes());
                            }
                        }

                        // Allocate new abandoned page
                        let new_addr = root.allocate_page();
                        if new_addr.raw() >= self.db.max_pages {
                            // Database full - can't track more abandoned pages
                            break;
                        }

                        // Create new abandoned page and link it
                        let mut new_ap = AbandonedPage::new(self.batch_id, self.batch_id);
                        new_ap.set_next(root.abandoned_head());
                        root.set_abandoned_head(new_addr);

                        // Add the address
                        new_ap.try_add(addr);

                        current_abandoned = Some(new_ap);
                        current_abandoned_addr = Some(new_addr);
                    }
                }

                // Write final abandoned page
                if let (Some(ap), Some(ap_addr)) = (current_abandoned, current_abandoned_addr) {
                    let mut mmap = self.db.mmap.lock();
                    let offset = ap_addr.file_offset() as usize;
                    if offset + PAGE_SIZE <= mmap.len() {
                        mmap[offset..offset + PAGE_SIZE].copy_from_slice(ap.into_page().as_bytes());
                    }
                }
            }
        }

        // Update root batch ID and get metadata for atomics
        let (block_number, block_hash, state_root) = {
            let mut root = self.db.root.write();
            let mut header = root.page().header();
            header.batch_id = self.batch_id;
            root.page_mut().set_header(header);
            (root.block_number(), root.block_hash(), root.state_root())
        };

        // Write root page
        self.db.write_root_internal()?;

        // Update atomics for lock-free readers (after root is written)
        self.db.block_number_atomic.store(block_number, Ordering::Release);
        self.db.state_root_atomic.store(state_root.raw(), Ordering::Release);
        self.db.block_hash_low.store(u64::from_le_bytes(block_hash[0..8].try_into().unwrap()), Ordering::Release);
        self.db.block_hash_mid1.store(u64::from_le_bytes(block_hash[8..16].try_into().unwrap()), Ordering::Release);
        self.db.block_hash_mid2.store(u64::from_le_bytes(block_hash[16..24].try_into().unwrap()), Ordering::Release);
        self.db.block_hash_high.store(u64::from_le_bytes(block_hash[24..32].try_into().unwrap()), Ordering::Release);

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

    #[test]
    fn test_abandoned_page_reuse() {
        let mut db = PagedDb::in_memory(100).unwrap();

        // Allocate a page
        let addr1 = {
            let mut batch = db.begin_batch();
            let (addr, _page) = batch.allocate_page(PageType::Data, 0).unwrap();
            batch.commit(CommitOptions::DangerNoFlush).unwrap();
            addr
        };

        // Abandon the page
        {
            let mut batch = db.begin_batch();
            batch.abandon_page(addr1);
            batch.commit(CommitOptions::DangerNoFlush).unwrap();
        }

        // Advance batches past the reorg depth (default 64)
        for _ in 0..65 {
            let batch = db.begin_batch();
            batch.commit(CommitOptions::DangerNoFlush).unwrap();
        }

        // Allocate again - should reuse the abandoned page
        let addr2 = {
            let mut batch = db.begin_batch();
            let (addr, _page) = batch.allocate_page(PageType::Leaf, 0).unwrap();
            batch.commit(CommitOptions::DangerNoFlush).unwrap();
            addr
        };

        assert_eq!(addr1, addr2, "Abandoned page should be reused");
    }

    #[test]
    fn test_abandoned_page_not_reused_before_reorg_depth() {
        let mut db = PagedDb::in_memory(100).unwrap();

        // Allocate a page
        let addr1 = {
            let mut batch = db.begin_batch();
            let (addr, _page) = batch.allocate_page(PageType::Data, 0).unwrap();
            batch.commit(CommitOptions::DangerNoFlush).unwrap();
            addr
        };

        // Abandon the page
        {
            let mut batch = db.begin_batch();
            batch.abandon_page(addr1);
            batch.commit(CommitOptions::DangerNoFlush).unwrap();
        }

        // Only advance a few batches (less than reorg depth)
        for _ in 0..5 {
            let batch = db.begin_batch();
            batch.commit(CommitOptions::DangerNoFlush).unwrap();
        }

        // Allocate again - should NOT reuse the abandoned page (too recent)
        let addr2 = {
            let mut batch = db.begin_batch();
            let (addr, _page) = batch.allocate_page(PageType::Leaf, 0).unwrap();
            batch.commit(CommitOptions::DangerNoFlush).unwrap();
            addr
        };

        assert_ne!(addr1, addr2, "Abandoned page should not be reused before reorg depth");
    }
}
