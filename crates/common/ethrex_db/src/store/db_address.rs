//! Database address representation.
//!
//! A DbAddress is a 32-bit value that represents a page address in the database.

use std::fmt;

/// Represents an address in the database.
///
/// The address is a 32-bit value where:
/// - Bits 0-31: Page number (supports up to 4 billion pages = 16TB with 4KB pages)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DbAddress(u32);

impl DbAddress {
    /// The null address (page 0 is reserved for metadata).
    pub const NULL: DbAddress = DbAddress(0);

    /// Size of the address in bytes.
    pub const SIZE: usize = std::mem::size_of::<u32>();

    /// Creates a new address pointing to a page.
    #[inline]
    pub const fn page(page_number: u32) -> Self {
        DbAddress(page_number)
    }

    /// Returns the raw value.
    #[inline]
    pub const fn raw(&self) -> u32 {
        self.0
    }

    /// Returns the file offset for this address.
    #[inline]
    pub const fn file_offset(&self) -> u64 {
        self.0 as u64 * super::PAGE_SIZE as u64
    }

    /// Returns true if this is the null address.
    #[inline]
    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    /// Returns the next address.
    #[inline]
    pub const fn next(&self) -> Self {
        DbAddress(self.0 + 1)
    }

    /// Reads an address from a byte slice (little-endian).
    pub fn read(data: &[u8]) -> Self {
        assert!(data.len() >= Self::SIZE);
        let value = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        DbAddress(value)
    }

    /// Writes the address to a byte slice (little-endian).
    pub fn write(&self, dest: &mut [u8]) {
        assert!(dest.len() >= Self::SIZE);
        dest[..Self::SIZE].copy_from_slice(&self.0.to_le_bytes());
    }
}

impl fmt::Debug for DbAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_null() {
            write!(f, "DbAddress(null)")
        } else {
            write!(f, "DbAddress(page={})", self.0)
        }
    }
}

impl fmt::Display for DbAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_null() {
            write!(f, "null")
        } else {
            write!(f, "Page @{}", self.0)
        }
    }
}

impl From<u32> for DbAddress {
    fn from(value: u32) -> Self {
        DbAddress(value)
    }
}

impl From<DbAddress> for u32 {
    fn from(addr: DbAddress) -> Self {
        addr.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_address() {
        let addr = DbAddress::NULL;
        assert!(addr.is_null());
        assert_eq!(addr.raw(), 0);
    }

    #[test]
    fn test_page_address() {
        let addr = DbAddress::page(42);
        assert!(!addr.is_null());
        assert_eq!(addr.raw(), 42);
        assert_eq!(addr.file_offset(), 42 * 4096);
    }

    #[test]
    fn test_read_write() {
        let addr = DbAddress::page(0x12345678);
        let mut buf = [0u8; 4];
        addr.write(&mut buf);
        let read = DbAddress::read(&buf);
        assert_eq!(addr, read);
    }

    #[test]
    fn test_next() {
        let addr = DbAddress::page(10);
        assert_eq!(addr.next(), DbAddress::page(11));
    }
}
