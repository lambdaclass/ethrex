//! NibblePath - Efficient path representation for trie traversal.
//!
//! A nibble is a half-byte (4 bits), representing values 0-15.
//! Ethereum trie paths are sequences of nibbles derived from keccak hashes.

/// Represents a path of nibbles for trie navigation.
///
/// Supports efficient slicing, comparison, and iteration over nibbles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NibblePath {
    /// Raw bytes containing the nibbles
    data: Vec<u8>,
    /// If true, the path starts at the high nibble of byte 0
    /// If false, the path starts at the low nibble of byte 0
    odd_start: bool,
    /// Number of nibbles in the path
    len: usize,
}

impl NibblePath {
    /// Creates a new empty NibblePath.
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            odd_start: false,
            len: 0,
        }
    }

    /// Creates a NibblePath from a byte slice.
    ///
    /// Each byte contains two nibbles: high nibble (bits 4-7) and low nibble (bits 0-3).
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            data: bytes.to_vec(),
            odd_start: false,
            len: bytes.len() * 2,
        }
    }

    /// Returns the number of nibbles in the path.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the path is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Gets the nibble at the given index.
    ///
    /// # Panics
    /// Panics if index >= len.
    pub fn get(&self, index: usize) -> u8 {
        assert!(index < self.len, "nibble index out of bounds");

        let adjusted_index = if self.odd_start { index + 1 } else { index };
        let byte_index = adjusted_index / 2;
        let is_high_nibble = adjusted_index % 2 == 0;

        if is_high_nibble {
            self.data[byte_index] >> 4
        } else {
            self.data[byte_index] & 0x0F
        }
    }

    /// Returns a slice of this path starting at the given nibble index.
    pub fn slice_from(&self, start: usize) -> Self {
        if start >= self.len {
            return Self::new();
        }

        let adjusted_start = if self.odd_start { start + 1 } else { start };
        let new_odd_start = adjusted_start % 2 == 1;
        let byte_start = adjusted_start / 2;

        Self {
            data: self.data[byte_start..].to_vec(),
            odd_start: new_odd_start,
            len: self.len - start,
        }
    }

    /// Returns a slice of the first `count` nibbles.
    pub fn slice_to(&self, count: usize) -> Self {
        if count >= self.len {
            return self.clone();
        }

        let adjusted_end = if self.odd_start { count + 1 } else { count };
        let byte_end = (adjusted_end + 1) / 2;

        Self {
            data: self.data[..byte_end].to_vec(),
            odd_start: self.odd_start,
            len: count,
        }
    }

    /// Returns the common prefix length with another path.
    pub fn common_prefix_len(&self, other: &Self) -> usize {
        let max_len = self.len.min(other.len);
        for i in 0..max_len {
            if self.get(i) != other.get(i) {
                return i;
            }
        }
        max_len
    }

    /// Returns an iterator over the nibbles.
    pub fn iter(&self) -> NibbleIterator<'_> {
        NibbleIterator {
            path: self,
            index: 0,
        }
    }
}

impl Default for NibblePath {
    fn default() -> Self {
        Self::new()
    }
}

/// Iterator over nibbles in a NibblePath.
pub struct NibbleIterator<'a> {
    path: &'a NibblePath,
    index: usize,
}

impl<'a> Iterator for NibbleIterator<'a> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.path.len {
            let nibble = self.path.get(self.index);
            self.index += 1;
            Some(nibble)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.path.len - self.index;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for NibbleIterator<'a> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_bytes() {
        let path = NibblePath::from_bytes(&[0xAB, 0xCD]);
        assert_eq!(path.len(), 4);
        assert_eq!(path.get(0), 0xA);
        assert_eq!(path.get(1), 0xB);
        assert_eq!(path.get(2), 0xC);
        assert_eq!(path.get(3), 0xD);
    }

    #[test]
    fn test_slice_from() {
        let path = NibblePath::from_bytes(&[0xAB, 0xCD]);
        let sliced = path.slice_from(1);
        assert_eq!(sliced.len(), 3);
        assert_eq!(sliced.get(0), 0xB);
        assert_eq!(sliced.get(1), 0xC);
        assert_eq!(sliced.get(2), 0xD);
    }

    #[test]
    fn test_common_prefix() {
        let path1 = NibblePath::from_bytes(&[0xAB, 0xCD]);
        let path2 = NibblePath::from_bytes(&[0xAB, 0xEF]);
        assert_eq!(path1.common_prefix_len(&path2), 2);
    }

    #[test]
    fn test_iterator() {
        let path = NibblePath::from_bytes(&[0xAB]);
        let nibbles: Vec<u8> = path.iter().collect();
        assert_eq!(nibbles, vec![0xA, 0xB]);
    }
}
