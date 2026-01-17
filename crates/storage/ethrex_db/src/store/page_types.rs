//! Specialized page types.
//!
//! Each page type provides specific functionality while sharing the common header.

use super::{DbAddress, Page, PageHeader, PageType, PAGE_SIZE};

/// Number of buckets in a data page (256 = 2 nibbles of fanout).
pub const BUCKET_COUNT: usize = 256;

// ============================================================================
// RootPage - Database metadata
// ============================================================================

/// Root page containing database metadata.
///
/// Layout after header:
/// - next_free_page: DbAddress (4 bytes)
/// - account_counter: u32 (4 bytes)
/// - state_root: DbAddress (4 bytes)
/// - block_number: u32 (4 bytes)
/// - block_hash: [u8; 32] (32 bytes)
/// - abandoned_pages: [DbAddress; ...] (remaining space)
pub struct RootPage {
    page: Page,
}

impl RootPage {
    /// Offset for next_free_page field.
    const NEXT_FREE_OFFSET: usize = PageHeader::SIZE;
    /// Offset for account_counter field.
    const ACCOUNT_COUNTER_OFFSET: usize = Self::NEXT_FREE_OFFSET + DbAddress::SIZE;
    /// Offset for state_root field.
    const STATE_ROOT_OFFSET: usize = Self::ACCOUNT_COUNTER_OFFSET + 4;
    /// Offset for block_number field.
    const BLOCK_NUMBER_OFFSET: usize = Self::STATE_ROOT_OFFSET + DbAddress::SIZE;
    /// Offset for block_hash field.
    const BLOCK_HASH_OFFSET: usize = Self::BLOCK_NUMBER_OFFSET + 4;
    /// Offset for abandoned page list head (linked list of AbandonedPages).
    const ABANDONED_HEAD_OFFSET: usize = Self::BLOCK_HASH_OFFSET + 32;
    /// Offset for reorg depth (number of batches before pages can be reused).
    const REORG_DEPTH_OFFSET: usize = Self::ABANDONED_HEAD_OFFSET + DbAddress::SIZE;
    /// Offset where inline abandoned page list starts (for small numbers of pages).
    const ABANDONED_LIST_OFFSET: usize = Self::REORG_DEPTH_OFFSET + 4;
    /// Offset for inline abandoned batch ID (when pages were abandoned).
    const ABANDONED_BATCH_OFFSET: usize = Self::ABANDONED_LIST_OFFSET;
    /// Offset for inline abandoned count.
    const ABANDONED_COUNT_OFFSET: usize = Self::ABANDONED_BATCH_OFFSET + 4;
    /// Offset where inline abandoned addresses start.
    const ABANDONED_ADDRESSES_OFFSET: usize = Self::ABANDONED_COUNT_OFFSET + 2;

    /// Maximum number of abandoned page addresses that can be stored inline.
    pub const MAX_ABANDONED: usize = (PAGE_SIZE - Self::ABANDONED_ADDRESSES_OFFSET) / DbAddress::SIZE;

    /// Default reorg depth (64 batches).
    pub const DEFAULT_REORG_DEPTH: u32 = 64;

    /// Creates a new root page.
    pub fn new(batch_id: u32) -> Self {
        let mut page = Page::new();
        page.set_header(PageHeader::new(batch_id, PageType::Root, 0));
        // Start allocating from page 1 (page 0 is the root)
        let mut root = Self { page };
        root.set_next_free_page(DbAddress::page(1));
        root.set_reorg_depth(Self::DEFAULT_REORG_DEPTH);
        root
    }

    /// Wraps an existing page as a root page.
    pub fn wrap(page: Page) -> Self {
        debug_assert_eq!(page.header().get_page_type(), Some(PageType::Root));
        Self { page }
    }

    /// Returns the underlying page.
    pub fn into_page(self) -> Page {
        self.page
    }

    /// Returns the underlying page reference.
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// Returns mutable underlying page reference.
    pub fn page_mut(&mut self) -> &mut Page {
        &mut self.page
    }

    /// Gets the next free page address.
    pub fn next_free_page(&self) -> DbAddress {
        let data = self.page.as_bytes();
        DbAddress::read(&data[Self::NEXT_FREE_OFFSET..])
    }

    /// Sets the next free page address.
    pub fn set_next_free_page(&mut self, addr: DbAddress) {
        let data = self.page.as_bytes_mut();
        addr.write(&mut data[Self::NEXT_FREE_OFFSET..]);
    }

    /// Allocates a new page and returns its address.
    pub fn allocate_page(&mut self) -> DbAddress {
        let addr = self.next_free_page();
        self.set_next_free_page(addr.next());
        addr
    }

    /// Gets the account counter.
    pub fn account_counter(&self) -> u32 {
        let data = self.page.as_bytes();
        u32::from_le_bytes([
            data[Self::ACCOUNT_COUNTER_OFFSET],
            data[Self::ACCOUNT_COUNTER_OFFSET + 1],
            data[Self::ACCOUNT_COUNTER_OFFSET + 2],
            data[Self::ACCOUNT_COUNTER_OFFSET + 3],
        ])
    }

    /// Increments and returns the account counter.
    pub fn next_account_id(&mut self) -> u32 {
        let current = self.account_counter();
        let next = current + 1;
        let data = self.page.as_bytes_mut();
        data[Self::ACCOUNT_COUNTER_OFFSET..Self::ACCOUNT_COUNTER_OFFSET + 4]
            .copy_from_slice(&next.to_le_bytes());
        next
    }

    /// Gets the state root address.
    pub fn state_root(&self) -> DbAddress {
        let data = self.page.as_bytes();
        DbAddress::read(&data[Self::STATE_ROOT_OFFSET..])
    }

    /// Sets the state root address.
    pub fn set_state_root(&mut self, addr: DbAddress) {
        let data = self.page.as_bytes_mut();
        addr.write(&mut data[Self::STATE_ROOT_OFFSET..]);
    }

    /// Gets the block number.
    pub fn block_number(&self) -> u32 {
        let data = self.page.as_bytes();
        u32::from_le_bytes([
            data[Self::BLOCK_NUMBER_OFFSET],
            data[Self::BLOCK_NUMBER_OFFSET + 1],
            data[Self::BLOCK_NUMBER_OFFSET + 2],
            data[Self::BLOCK_NUMBER_OFFSET + 3],
        ])
    }

    /// Sets the block number.
    pub fn set_block_number(&mut self, number: u32) {
        let data = self.page.as_bytes_mut();
        data[Self::BLOCK_NUMBER_OFFSET..Self::BLOCK_NUMBER_OFFSET + 4]
            .copy_from_slice(&number.to_le_bytes());
    }

    /// Gets the block hash.
    pub fn block_hash(&self) -> [u8; 32] {
        let data = self.page.as_bytes();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&data[Self::BLOCK_HASH_OFFSET..Self::BLOCK_HASH_OFFSET + 32]);
        hash
    }

    /// Sets the block hash.
    pub fn set_block_hash(&mut self, hash: &[u8; 32]) {
        let data = self.page.as_bytes_mut();
        data[Self::BLOCK_HASH_OFFSET..Self::BLOCK_HASH_OFFSET + 32].copy_from_slice(hash);
    }

    /// Gets the head of the abandoned page linked list.
    pub fn abandoned_head(&self) -> DbAddress {
        let data = self.page.as_bytes();
        DbAddress::read(&data[Self::ABANDONED_HEAD_OFFSET..])
    }

    /// Sets the head of the abandoned page linked list.
    pub fn set_abandoned_head(&mut self, addr: DbAddress) {
        let data = self.page.as_bytes_mut();
        addr.write(&mut data[Self::ABANDONED_HEAD_OFFSET..]);
    }

    /// Gets the reorg depth (number of batches before pages can be reused).
    pub fn reorg_depth(&self) -> u32 {
        let data = self.page.as_bytes();
        u32::from_le_bytes([
            data[Self::REORG_DEPTH_OFFSET],
            data[Self::REORG_DEPTH_OFFSET + 1],
            data[Self::REORG_DEPTH_OFFSET + 2],
            data[Self::REORG_DEPTH_OFFSET + 3],
        ])
    }

    /// Sets the reorg depth.
    pub fn set_reorg_depth(&mut self, depth: u32) {
        let data = self.page.as_bytes_mut();
        data[Self::REORG_DEPTH_OFFSET..Self::REORG_DEPTH_OFFSET + 4]
            .copy_from_slice(&depth.to_le_bytes());
    }

    /// Gets the batch ID when inline abandoned pages were abandoned.
    fn abandoned_batch(&self) -> u32 {
        let data = self.page.as_bytes();
        u32::from_le_bytes([
            data[Self::ABANDONED_BATCH_OFFSET],
            data[Self::ABANDONED_BATCH_OFFSET + 1],
            data[Self::ABANDONED_BATCH_OFFSET + 2],
            data[Self::ABANDONED_BATCH_OFFSET + 3],
        ])
    }

    /// Sets the batch ID when inline abandoned pages were abandoned.
    fn set_abandoned_batch(&mut self, batch: u32) {
        let data = self.page.as_bytes_mut();
        data[Self::ABANDONED_BATCH_OFFSET..Self::ABANDONED_BATCH_OFFSET + 4]
            .copy_from_slice(&batch.to_le_bytes());
    }

    /// Gets the inline abandoned page count.
    fn abandoned_count(&self) -> usize {
        let data = self.page.as_bytes();
        u16::from_le_bytes([
            data[Self::ABANDONED_COUNT_OFFSET],
            data[Self::ABANDONED_COUNT_OFFSET + 1],
        ]) as usize
    }

    /// Sets the inline abandoned page count.
    fn set_abandoned_count(&mut self, count: usize) {
        let data = self.page.as_bytes_mut();
        let count16 = count as u16;
        data[Self::ABANDONED_COUNT_OFFSET..Self::ABANDONED_COUNT_OFFSET + 2]
            .copy_from_slice(&count16.to_le_bytes());
    }

    /// Tries to add an abandoned page address inline (fast path).
    /// Returns false if inline storage is full or batch mismatch.
    pub fn try_add_abandoned_inline(&mut self, addr: DbAddress, batch_id: u32) -> bool {
        let count = self.abandoned_count();

        // If first entry, set the batch
        if count == 0 {
            self.set_abandoned_batch(batch_id);
        } else if self.abandoned_batch() != batch_id {
            // Different batch - would need to use AbandonedPage linked list
            return false;
        }

        if count >= Self::MAX_ABANDONED {
            return false;
        }

        let offset = Self::ABANDONED_ADDRESSES_OFFSET + count * DbAddress::SIZE;
        addr.write(&mut self.page.as_bytes_mut()[offset..]);
        self.set_abandoned_count(count + 1);
        true
    }

    /// Pops an abandoned page address from inline storage (fast path).
    /// Only returns pages that are safe to reuse (past reorg depth).
    pub fn pop_abandoned_inline(&mut self, current_batch: u32) -> Option<DbAddress> {
        let count = self.abandoned_count();
        if count == 0 {
            return None;
        }

        // Check if enough batches have passed
        let abandoned_at = self.abandoned_batch();
        let reorg_depth = self.reorg_depth();
        if current_batch < abandoned_at + reorg_depth {
            // Not old enough to reuse
            return None;
        }

        let offset = Self::ABANDONED_ADDRESSES_OFFSET + (count - 1) * DbAddress::SIZE;
        let addr = DbAddress::read(&self.page.as_bytes()[offset..]);
        self.set_abandoned_count(count - 1);
        Some(addr)
    }

    /// Returns true if there are inline abandoned pages that can be reused.
    pub fn has_reusable_abandoned_inline(&self, current_batch: u32) -> bool {
        let count = self.abandoned_count();
        if count == 0 {
            return false;
        }

        let abandoned_at = self.abandoned_batch();
        let reorg_depth = self.reorg_depth();
        current_batch >= abandoned_at + reorg_depth
    }
}

// ============================================================================
// DataPage - Intermediate node with fanout
// ============================================================================

/// Data page with fanout buckets and inline storage.
///
/// Layout after header:
/// - buckets: [DbAddress; 256] (1024 bytes for fanout)
/// - merkle_addr: DbAddress (4 bytes)
/// - child_data_pages: u32 (4 bytes, bitmap for fanout optimization)
/// - data: SlottedArray (remaining space)
pub struct DataPage {
    page: Page,
}

impl DataPage {
    /// Offset for buckets array.
    const BUCKETS_OFFSET: usize = PageHeader::SIZE;
    /// Size of the buckets array (256 * 4 bytes).
    const BUCKETS_SIZE: usize = BUCKET_COUNT * DbAddress::SIZE;
    /// Offset for merkle address.
    const MERKLE_OFFSET: usize = Self::BUCKETS_OFFSET + Self::BUCKETS_SIZE;
    /// Offset for child data pages bitmap.
    const CHILD_BITMAP_OFFSET: usize = Self::MERKLE_OFFSET + DbAddress::SIZE;
    /// Offset where data storage starts.
    const DATA_OFFSET: usize = Self::CHILD_BITMAP_OFFSET + 4;

    /// Creates a new data page.
    pub fn new(batch_id: u32, level: u8) -> Self {
        let mut page = Page::new();
        page.set_header(PageHeader::new(batch_id, PageType::Data, level));
        Self { page }
    }

    /// Wraps an existing page as a data page.
    pub fn wrap(page: Page) -> Self {
        debug_assert_eq!(page.header().get_page_type(), Some(PageType::Data));
        Self { page }
    }

    /// Returns the underlying page.
    pub fn into_page(self) -> Page {
        self.page
    }

    /// Returns a reference to the underlying page.
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// Returns a mutable reference to the underlying page.
    pub fn page_mut(&mut self) -> &mut Page {
        &mut self.page
    }

    /// Gets a bucket address by index.
    pub fn get_bucket(&self, index: usize) -> DbAddress {
        debug_assert!(index < BUCKET_COUNT);
        let offset = Self::BUCKETS_OFFSET + index * DbAddress::SIZE;
        DbAddress::read(&self.page.as_bytes()[offset..])
    }

    /// Sets a bucket address.
    pub fn set_bucket(&mut self, index: usize, addr: DbAddress) {
        debug_assert!(index < BUCKET_COUNT);
        let offset = Self::BUCKETS_OFFSET + index * DbAddress::SIZE;
        addr.write(&mut self.page.as_bytes_mut()[offset..]);
    }

    /// Gets the merkle sidecar page address.
    pub fn merkle_addr(&self) -> DbAddress {
        DbAddress::read(&self.page.as_bytes()[Self::MERKLE_OFFSET..])
    }

    /// Sets the merkle sidecar page address.
    pub fn set_merkle_addr(&mut self, addr: DbAddress) {
        addr.write(&mut self.page.as_bytes_mut()[Self::MERKLE_OFFSET..]);
    }

    /// Returns whether this page uses full fanout (256 buckets consuming 2 nibbles).
    pub fn is_fanout(&self) -> bool {
        let data = self.page.as_bytes();
        let bitmap = u32::from_le_bytes([
            data[Self::CHILD_BITMAP_OFFSET],
            data[Self::CHILD_BITMAP_OFFSET + 1],
            data[Self::CHILD_BITMAP_OFFSET + 2],
            data[Self::CHILD_BITMAP_OFFSET + 3],
        ]);
        bitmap == 0xFFFF
    }

    /// Returns the number of nibbles consumed by this page.
    pub fn consumed_nibbles(&self) -> usize {
        if self.is_fanout() { 2 } else { 1 }
    }

    /// Returns the bucket index for a given nibble path.
    pub fn bucket_for_path(&self, nibble0: u8, nibble1: Option<u8>) -> usize {
        if self.is_fanout() {
            let n1 = nibble1.unwrap_or(0);
            ((nibble0 as usize) << 4) | (n1 as usize)
        } else {
            nibble0 as usize
        }
    }

    /// Returns a slotted array view of the data area.
    pub fn data(&self) -> &[u8] {
        &self.page.as_bytes()[Self::DATA_OFFSET..]
    }

    /// Returns a mutable slotted array view of the data area.
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.page.as_bytes_mut()[Self::DATA_OFFSET..]
    }
}

// ============================================================================
// LeafPage - Bottom of the tree
// ============================================================================

/// Leaf page at the bottom of the tree, using slotted array storage.
///
/// Layout after header:
/// - data: SlottedArray (entire payload)
pub struct LeafPage {
    page: Page,
}

impl LeafPage {
    /// Creates a new leaf page.
    pub fn new(batch_id: u32, level: u8) -> Self {
        let mut page = Page::new();
        page.set_header(PageHeader::new(batch_id, PageType::Leaf, level));
        Self { page }
    }

    /// Wraps an existing page as a leaf page.
    pub fn wrap(page: Page) -> Self {
        debug_assert_eq!(page.header().get_page_type(), Some(PageType::Leaf));
        Self { page }
    }

    /// Returns the underlying page.
    pub fn into_page(self) -> Page {
        self.page
    }

    /// Returns a reference to the underlying page.
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// Returns a mutable reference to the underlying page.
    pub fn page_mut(&mut self) -> &mut Page {
        &mut self.page
    }

    /// Returns the data area for slotted array storage.
    pub fn data(&self) -> &[u8] {
        self.page.payload()
    }

    /// Returns mutable data area.
    pub fn data_mut(&mut self) -> &mut [u8] {
        self.page.payload_mut()
    }
}

// ============================================================================
// AbandonedPage - Tracking reusable pages
// ============================================================================

/// Page tracking abandoned pages that can be reused.
///
/// When a page is COWed, the original is tracked here for later reuse
/// (after the reorg depth has passed).
///
/// Layout after header:
/// - batch_id_abandoned: u32 (4 bytes) - batch when pages were abandoned
/// - next_abandoned: DbAddress (4 bytes) - linked list of abandoned pages
/// - count: u16 (2 bytes) - number of addresses stored
/// - addresses: [DbAddress; ...] (remaining space)
pub struct AbandonedPage {
    page: Page,
}

impl AbandonedPage {
    const BATCH_ABANDONED_OFFSET: usize = PageHeader::SIZE;
    const NEXT_OFFSET: usize = Self::BATCH_ABANDONED_OFFSET + 4;
    const COUNT_OFFSET: usize = Self::NEXT_OFFSET + DbAddress::SIZE;
    const ADDRESSES_OFFSET: usize = Self::COUNT_OFFSET + 2;

    /// Maximum addresses that can be stored.
    pub const MAX_ADDRESSES: usize = (PAGE_SIZE - Self::ADDRESSES_OFFSET) / DbAddress::SIZE;

    /// Creates a new abandoned page.
    pub fn new(batch_id: u32, abandoned_at_batch: u32) -> Self {
        let mut page = Page::new();
        page.set_header(PageHeader::new(batch_id, PageType::Abandoned, 0));
        let mut ap = Self { page };
        ap.set_batch_abandoned(abandoned_at_batch);
        ap
    }

    /// Wraps an existing page.
    pub fn wrap(page: Page) -> Self {
        debug_assert_eq!(page.header().get_page_type(), Some(PageType::Abandoned));
        Self { page }
    }

    /// Returns the underlying page.
    pub fn into_page(self) -> Page {
        self.page
    }

    /// Gets the batch when these pages were abandoned.
    pub fn batch_abandoned(&self) -> u32 {
        let data = self.page.as_bytes();
        u32::from_le_bytes([
            data[Self::BATCH_ABANDONED_OFFSET],
            data[Self::BATCH_ABANDONED_OFFSET + 1],
            data[Self::BATCH_ABANDONED_OFFSET + 2],
            data[Self::BATCH_ABANDONED_OFFSET + 3],
        ])
    }

    /// Sets the batch when these pages were abandoned.
    fn set_batch_abandoned(&mut self, batch: u32) {
        let data = self.page.as_bytes_mut();
        data[Self::BATCH_ABANDONED_OFFSET..Self::BATCH_ABANDONED_OFFSET + 4]
            .copy_from_slice(&batch.to_le_bytes());
    }

    /// Gets the next abandoned page in the linked list.
    pub fn next(&self) -> DbAddress {
        DbAddress::read(&self.page.as_bytes()[Self::NEXT_OFFSET..])
    }

    /// Sets the next abandoned page.
    pub fn set_next(&mut self, addr: DbAddress) {
        addr.write(&mut self.page.as_bytes_mut()[Self::NEXT_OFFSET..]);
    }

    /// Gets the number of addresses stored.
    pub fn count(&self) -> usize {
        let data = self.page.as_bytes();
        u16::from_le_bytes([data[Self::COUNT_OFFSET], data[Self::COUNT_OFFSET + 1]]) as usize
    }

    /// Tries to add an abandoned page address.
    pub fn try_add(&mut self, addr: DbAddress) -> bool {
        let count = self.count();
        if count >= Self::MAX_ADDRESSES {
            return false;
        }

        let offset = Self::ADDRESSES_OFFSET + count * DbAddress::SIZE;
        addr.write(&mut self.page.as_bytes_mut()[offset..]);

        let data = self.page.as_bytes_mut();
        let new_count = (count + 1) as u16;
        data[Self::COUNT_OFFSET..Self::COUNT_OFFSET + 2].copy_from_slice(&new_count.to_le_bytes());

        true
    }

    /// Gets an address at the given index.
    pub fn get(&self, index: usize) -> Option<DbAddress> {
        if index >= self.count() {
            return None;
        }
        let offset = Self::ADDRESSES_OFFSET + index * DbAddress::SIZE;
        Some(DbAddress::read(&self.page.as_bytes()[offset..]))
    }

    /// Pops the last address.
    pub fn pop(&mut self) -> Option<DbAddress> {
        let count = self.count();
        if count == 0 {
            return None;
        }

        let addr = self.get(count - 1)?;

        let data = self.page.as_bytes_mut();
        let new_count = (count - 1) as u16;
        data[Self::COUNT_OFFSET..Self::COUNT_OFFSET + 2].copy_from_slice(&new_count.to_le_bytes());

        Some(addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_page() {
        let mut root = RootPage::new(1);
        assert_eq!(root.next_free_page(), DbAddress::page(1));

        let addr = root.allocate_page();
        assert_eq!(addr, DbAddress::page(1));
        assert_eq!(root.next_free_page(), DbAddress::page(2));

        root.set_block_number(42);
        assert_eq!(root.block_number(), 42);
    }

    #[test]
    fn test_data_page() {
        let mut dp = DataPage::new(1, 0);

        dp.set_bucket(0, DbAddress::page(10));
        dp.set_bucket(255, DbAddress::page(20));

        assert_eq!(dp.get_bucket(0), DbAddress::page(10));
        assert_eq!(dp.get_bucket(255), DbAddress::page(20));
        assert_eq!(dp.get_bucket(1), DbAddress::NULL);
    }

    #[test]
    fn test_abandoned_page() {
        let mut ap = AbandonedPage::new(1, 5);
        assert_eq!(ap.batch_abandoned(), 5);
        assert_eq!(ap.count(), 0);

        assert!(ap.try_add(DbAddress::page(10)));
        assert!(ap.try_add(DbAddress::page(20)));
        assert_eq!(ap.count(), 2);

        assert_eq!(ap.pop(), Some(DbAddress::page(20)));
        assert_eq!(ap.count(), 1);
    }
}
