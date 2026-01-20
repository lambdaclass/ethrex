//! Page header shared across all page types.

use std::mem;

/// The current version of the page format.
pub const CURRENT_VERSION: u8 = 1;

/// Types of pages in the database.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PageType {
    /// Root page containing database metadata.
    Root = 0,
    /// Data page with fanout buckets and inline storage.
    Data = 1,
    /// Leaf page (bottom of the tree) with slotted array storage.
    Leaf = 2,
    /// Page tracking abandoned pages for reuse.
    Abandoned = 3,
    /// State root page (top-level for state trie).
    StateRoot = 4,
    /// Storage fanout page.
    StorageFanout = 5,
}

impl PageType {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(PageType::Root),
            1 => Some(PageType::Data),
            2 => Some(PageType::Leaf),
            3 => Some(PageType::Abandoned),
            4 => Some(PageType::StateRoot),
            5 => Some(PageType::StorageFanout),
            _ => None,
        }
    }
}

/// Header shared across all pages.
///
/// Layout (8 bytes):
/// - batch_id: u32 - ID of the last batch that wrote to this page
/// - version: u8 - Page format version
/// - page_type: u8 - Type of the page
/// - level: u8 - Depth in the tree
/// - metadata: u8 - Reserved for page-specific metadata
#[derive(Clone, Copy, Debug)]
#[repr(C, packed)]
pub struct PageHeader {
    /// The ID of the last batch that wrote to this page.
    pub batch_id: u32,
    /// The version of the page format.
    pub version: u8,
    /// The type of the page.
    pub page_type: u8,
    /// The depth/level in the tree.
    pub level: u8,
    /// Reserved metadata byte.
    pub metadata: u8,
}

impl PageHeader {
    /// Size of the header in bytes.
    pub const SIZE: usize = mem::size_of::<Self>();

    /// Creates a new page header.
    pub fn new(batch_id: u32, page_type: PageType, level: u8) -> Self {
        Self {
            batch_id,
            version: CURRENT_VERSION,
            page_type: page_type as u8,
            level,
            metadata: 0,
        }
    }

    /// Returns the page type.
    pub fn get_page_type(&self) -> Option<PageType> {
        PageType::from_u8(self.page_type)
    }

    /// Reads a header from a byte slice.
    pub fn read(data: &[u8]) -> Self {
        assert!(data.len() >= Self::SIZE);
        unsafe { std::ptr::read(data.as_ptr() as *const Self) }
    }

    /// Writes the header to a byte slice.
    pub fn write(&self, dest: &mut [u8]) {
        assert!(dest.len() >= Self::SIZE);
        unsafe {
            std::ptr::write(dest.as_mut_ptr() as *mut Self, *self);
        }
    }
}

impl Default for PageHeader {
    fn default() -> Self {
        Self {
            batch_id: 0,
            version: CURRENT_VERSION,
            page_type: PageType::Data as u8,
            level: 0,
            metadata: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_size() {
        assert_eq!(PageHeader::SIZE, 8);
    }

    #[test]
    fn test_header_read_write() {
        let header = PageHeader::new(42, PageType::Data, 3);
        let mut buf = [0u8; PageHeader::SIZE];
        header.write(&mut buf);

        let read = PageHeader::read(&buf);
        // Copy fields to avoid packed struct reference issues
        let batch_id = read.batch_id;
        let level = read.level;
        assert_eq!(batch_id, 42);
        assert_eq!(read.get_page_type(), Some(PageType::Data));
        assert_eq!(level, 3);
    }
}
