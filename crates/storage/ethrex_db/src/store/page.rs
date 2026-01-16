//! Base page abstraction.
//!
//! All pages in the database are 4KB and share a common header.

use super::page_header::PageHeader;

/// Page size in bytes (4KB).
pub const PAGE_SIZE: usize = 4096;

/// A raw page in the database.
///
/// This is a thin wrapper around a byte array representing a 4KB page.
#[derive(Clone)]
pub struct Page {
    data: Box<[u8; PAGE_SIZE]>,
}

impl Page {
    /// Creates a new zeroed page.
    pub fn new() -> Self {
        Self {
            data: Box::new([0u8; PAGE_SIZE]),
        }
    }

    /// Creates a page from existing data.
    pub fn from_bytes(data: [u8; PAGE_SIZE]) -> Self {
        Self {
            data: Box::new(data),
        }
    }

    /// Returns a reference to the raw bytes.
    pub fn as_bytes(&self) -> &[u8; PAGE_SIZE] {
        &self.data
    }

    /// Returns a mutable reference to the raw bytes.
    pub fn as_bytes_mut(&mut self) -> &mut [u8; PAGE_SIZE] {
        &mut self.data
    }

    /// Returns a reference to the page header.
    pub fn header(&self) -> PageHeader {
        PageHeader::read(&self.data[..])
    }

    /// Sets the page header.
    pub fn set_header(&mut self, header: PageHeader) {
        header.write(&mut self.data[..]);
    }

    /// Returns the payload portion of the page (after the header).
    pub fn payload(&self) -> &[u8] {
        &self.data[PageHeader::SIZE..]
    }

    /// Returns a mutable reference to the payload.
    pub fn payload_mut(&mut self) -> &mut [u8] {
        &mut self.data[PageHeader::SIZE..]
    }

    /// Clears the page (zeroes all bytes).
    pub fn clear(&mut self) {
        self.data.fill(0);
    }

    /// Copies this page to another page.
    pub fn copy_to(&self, other: &mut Page) {
        other.data.copy_from_slice(&*self.data);
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let header = self.header();
        // Copy fields to avoid packed struct reference issues
        let batch_id = header.batch_id;
        let page_type = header.get_page_type();
        let level = header.level;
        f.debug_struct("Page")
            .field("batch_id", &batch_id)
            .field("page_type", &page_type)
            .field("level", &level)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::PageType;

    #[test]
    fn test_page_new() {
        let page = Page::new();
        assert_eq!(page.as_bytes().len(), PAGE_SIZE);
    }

    #[test]
    fn test_page_header() {
        let mut page = Page::new();
        let header = PageHeader::new(42, PageType::Data, 5);
        page.set_header(header);

        let read = page.header();
        // Copy fields to avoid packed struct reference issues
        let batch_id = read.batch_id;
        let level = read.level;
        assert_eq!(batch_id, 42);
        assert_eq!(level, 5);
    }

    #[test]
    fn test_payload_size() {
        let page = Page::new();
        assert_eq!(page.payload().len(), PAGE_SIZE - PageHeader::SIZE);
    }
}
