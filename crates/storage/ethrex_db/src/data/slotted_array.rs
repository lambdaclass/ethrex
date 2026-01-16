//! SlottedArray - In-page key-value storage using the slot array pattern.
//!
//! Inspired by PostgreSQL's page layout, this structure stores variable-length
//! entries in a fixed-size buffer by growing slots from the start and data
//! from the end.

use super::NibblePath;

/// Fixed page size in bytes (4KB).
pub const PAGE_SIZE: usize = 4096;

/// Header size at the start of the slotted array.
const HEADER_SIZE: usize = 8;

/// Size of each slot entry.
const SLOT_SIZE: usize = 4;

/// A slotted array for storing key-value pairs in a fixed-size page.
///
/// Layout:
/// ```text
/// [Header (8 bytes)][Slots...] ... free space ... [Data...]
/// ```
///
/// - Header: tracks slot count and data pointer
/// - Slots grow forward from after header
/// - Data grows backward from end of page
pub struct SlottedArray {
    data: [u8; PAGE_SIZE],
}

/// Header stored at the beginning of the array.
#[repr(C)]
#[derive(Clone, Copy)]
struct Header {
    /// Number of slots (including deleted)
    slot_count: u16,
    /// Offset where data region starts (grows downward from PAGE_SIZE)
    data_start: u16,
    /// Reserved for future use
    _reserved: u32,
}

/// A slot entry pointing to stored data.
#[repr(C)]
#[derive(Clone, Copy)]
struct Slot {
    /// Offset to the data from the start of the page
    offset: u16,
    /// Length of the key + value data
    length: u16,
}

impl SlottedArray {
    /// Creates a new empty SlottedArray.
    pub fn new() -> Self {
        let mut arr = Self {
            data: [0; PAGE_SIZE],
        };
        arr.set_header(Header {
            slot_count: 0,
            data_start: PAGE_SIZE as u16,
            _reserved: 0,
        });
        arr
    }

    /// Creates a SlottedArray from an existing page buffer.
    pub fn from_bytes(data: [u8; PAGE_SIZE]) -> Self {
        Self { data }
    }

    /// Returns the underlying bytes.
    pub fn as_bytes(&self) -> &[u8; PAGE_SIZE] {
        &self.data
    }

    /// Returns the number of entries (including tombstones).
    pub fn slot_count(&self) -> usize {
        self.header().slot_count as usize
    }

    /// Returns the amount of free space available.
    pub fn free_space(&self) -> usize {
        let header = self.header();
        let slots_end = HEADER_SIZE + (header.slot_count as usize * SLOT_SIZE);
        let data_start = header.data_start as usize;

        if data_start > slots_end {
            data_start - slots_end
        } else {
            0
        }
    }

    /// Tries to insert a key-value pair.
    ///
    /// Returns `true` if successful, `false` if there's not enough space.
    pub fn try_insert(&mut self, key: &NibblePath, value: &[u8]) -> bool {
        let key_bytes = self.encode_key(key);
        let entry_size = key_bytes.len() + value.len();
        let required_space = SLOT_SIZE + entry_size;

        if self.free_space() < required_space {
            return false;
        }

        let mut header = self.header();

        // Allocate space for data (grows downward)
        let data_offset = header.data_start as usize - entry_size;

        // Write key and value
        self.data[data_offset..data_offset + key_bytes.len()].copy_from_slice(&key_bytes);
        self.data[data_offset + key_bytes.len()..data_offset + entry_size].copy_from_slice(value);

        // Write slot
        let slot_offset = HEADER_SIZE + (header.slot_count as usize * SLOT_SIZE);
        let slot = Slot {
            offset: data_offset as u16,
            length: entry_size as u16,
        };
        self.set_slot(slot_offset, slot);

        // Update header
        header.slot_count += 1;
        header.data_start = data_offset as u16;
        self.set_header(header);

        true
    }

    /// Looks up a value by key.
    ///
    /// Returns `Some(value)` if found, `None` if not present.
    /// Searches from the end (newest entries first) so that newer inserts
    /// take precedence over older ones with the same key.
    pub fn get(&self, key: &NibblePath) -> Option<Vec<u8>> {
        let key_bytes = self.encode_key(key);
        let header = self.header();
        let slot_count = header.slot_count as usize;

        // Search from newest to oldest (end to start)
        for i in (0..slot_count).rev() {
            let slot = self.get_slot(HEADER_SIZE + i * SLOT_SIZE);

            // Skip tombstones (offset == 0 and length == 0)
            if slot.offset == 0 && slot.length == 0 {
                continue;
            }

            let entry_start = slot.offset as usize;
            let entry_end = entry_start + slot.length as usize;
            let entry = &self.data[entry_start..entry_end];

            // Check if key matches
            if entry.starts_with(&key_bytes) {
                let value_start = key_bytes.len();
                return Some(entry[value_start..].to_vec());
            }
        }

        None
    }

    /// Marks an entry as deleted (tombstone).
    ///
    /// Deletes the most recent entry with the given key (searches from end).
    /// Returns `true` if the key was found and deleted.
    pub fn delete(&mut self, key: &NibblePath) -> bool {
        let key_bytes = self.encode_key(key);
        let header = self.header();
        let slot_count = header.slot_count as usize;

        // Search from newest to oldest (end to start)
        for i in (0..slot_count).rev() {
            let slot_offset = HEADER_SIZE + i * SLOT_SIZE;
            let slot = self.get_slot(slot_offset);

            if slot.offset == 0 && slot.length == 0 {
                continue;
            }

            let entry_start = slot.offset as usize;
            let entry_end = entry_start + slot.length as usize;
            let entry = &self.data[entry_start..entry_end];

            if entry.starts_with(&key_bytes) {
                // Mark as tombstone
                self.set_slot(slot_offset, Slot { offset: 0, length: 0 });
                return true;
            }
        }

        false
    }

    /// Encodes a NibblePath as bytes for storage.
    fn encode_key(&self, key: &NibblePath) -> Vec<u8> {
        // Simple encoding: length byte + raw nibble data
        let mut encoded = Vec::with_capacity(1 + (key.len() + 1) / 2);
        encoded.push(key.len() as u8);

        // Pack nibbles into bytes
        let mut i = 0;
        while i < key.len() {
            let high = key.get(i);
            let low = if i + 1 < key.len() {
                key.get(i + 1)
            } else {
                0
            };
            encoded.push((high << 4) | low);
            i += 2;
        }

        encoded
    }

    fn header(&self) -> Header {
        unsafe { std::ptr::read(self.data.as_ptr() as *const Header) }
    }

    fn set_header(&mut self, header: Header) {
        unsafe {
            std::ptr::write(self.data.as_mut_ptr() as *mut Header, header);
        }
    }

    fn get_slot(&self, offset: usize) -> Slot {
        unsafe { std::ptr::read(self.data.as_ptr().add(offset) as *const Slot) }
    }

    fn set_slot(&mut self, offset: usize, slot: Slot) {
        unsafe {
            std::ptr::write(self.data.as_mut_ptr().add(offset) as *mut Slot, slot);
        }
    }
}

impl Default for SlottedArray {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_array() {
        let arr = SlottedArray::new();
        assert_eq!(arr.slot_count(), 0);
        assert!(arr.free_space() > 0);
    }

    #[test]
    fn test_insert_and_get() {
        let mut arr = SlottedArray::new();
        let key = NibblePath::from_bytes(&[0xAB, 0xCD]);
        let value = b"hello world";

        assert!(arr.try_insert(&key, value));
        assert_eq!(arr.slot_count(), 1);

        let retrieved = arr.get(&key);
        assert_eq!(retrieved, Some(value.to_vec()));
    }

    #[test]
    fn test_delete() {
        let mut arr = SlottedArray::new();
        let key = NibblePath::from_bytes(&[0xAB, 0xCD]);
        let value = b"test";

        arr.try_insert(&key, value);
        assert!(arr.get(&key).is_some());

        assert!(arr.delete(&key));
        assert!(arr.get(&key).is_none());
    }

    #[test]
    fn test_multiple_entries() {
        let mut arr = SlottedArray::new();

        for i in 0..10 {
            let key = NibblePath::from_bytes(&[i, i + 1]);
            let value = format!("value_{}", i);
            assert!(arr.try_insert(&key, value.as_bytes()));
        }

        assert_eq!(arr.slot_count(), 10);

        for i in 0..10 {
            let key = NibblePath::from_bytes(&[i, i + 1]);
            let expected = format!("value_{}", i);
            assert_eq!(arr.get(&key), Some(expected.into_bytes()));
        }
    }
}
