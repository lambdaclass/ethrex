use std::{cmp, mem};

use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

// TODO: move path-tracking logic somewhere else
/// Struct representing a list of nibbles (half-bytes), packed two nibbles per byte
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Deserialize,
    rkyv::Serialize,
    rkyv::Archive,
)]
pub struct Nibbles {
    /// Packed nibble data: two nibbles per byte (high nibble in upper 4 bits, low nibble in lower 4 bits)
    data: Vec<u8>,
    /// Actual number of nibbles stored (handles odd-length sequences)
    len: usize,
    /// Whether this nibble sequence represents a leaf node (replaces the magic value 16)
    is_leaf: bool,
    /// Parts of the path that have already been consumed (used for tracking
    /// current position when visiting nodes). See `current()`. Also packed.
    already_consumed: Vec<u8>,
    /// Number of nibbles in already_consumed
    consumed_len: usize,
}

impl Default for Nibbles {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            len: 0,
            is_leaf: false,
            already_consumed: Vec::new(),
            consumed_len: 0,
        }
    }
}

// NOTE: custom impls to ignore the `already_consumed` field

impl PartialEq for Nibbles {
    fn eq(&self, other: &Nibbles) -> bool {
        // Compare nibble by nibble to ensure correct comparison
        // Note: is_leaf is not compared (it's metadata, not part of the nibble sequence)
        if self.len != other.len {
            return false;
        }

        for i in 0..self.len {
            if Self::get_nibble(&self.data, i) != Self::get_nibble(&other.data, i) {
                return false;
            }
        }

        true
    }
}

impl Eq for Nibbles {}

impl PartialOrd for Nibbles {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Nibbles {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        // Compare nibble by nibble
        let min_len = self.len.min(other.len);

        for i in 0..min_len {
            match Self::get_nibble(&self.data, i).cmp(&Self::get_nibble(&other.data, i)) {
                cmp::Ordering::Equal => continue,
                other => return other,
            }
        }

        // If all nibbles match up to min_len, compare lengths
        match self.len.cmp(&other.len) {
            cmp::Ordering::Equal => {
                // If lengths are equal, compare leaf flags
                self.is_leaf.cmp(&other.is_leaf)
            }
            other => other,
        }
    }
}

impl std::hash::Hash for Nibbles {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.data.hash(state);
        self.len.hash(state);
        self.is_leaf.hash(state);
    }
}

// Helper functions for nibble packing/unpacking
impl Nibbles {
    /// Get a single nibble at the given index from packed data
    #[inline]
    fn get_nibble(data: &[u8], index: usize) -> u8 {
        let byte_idx = index / 2;
        if index % 2 == 0 {
            // High nibble (upper 4 bits)
            (data[byte_idx] >> 4) & 0x0F
        } else {
            // Low nibble (lower 4 bits)
            data[byte_idx] & 0x0F
        }
    }

    /// Set a single nibble at the given index in packed data
    #[inline]
    fn set_nibble(data: &mut Vec<u8>, index: usize, value: u8) {
        let byte_idx = index / 2;
        // Ensure we have enough capacity
        if byte_idx >= data.len() {
            data.resize(byte_idx + 1, 0);
        }
        if index % 2 == 0 {
            // High nibble (upper 4 bits)
            data[byte_idx] = (data[byte_idx] & 0x0F) | ((value & 0x0F) << 4);
        } else {
            // Low nibble (lower 4 bits)
            data[byte_idx] = (data[byte_idx] & 0xF0) | (value & 0x0F);
        }
    }

    /// Pack a sequence of nibbles into bytes (2 nibbles per byte)
    fn pack_nibbles(nibbles: &[u8]) -> (Vec<u8>, usize) {
        let len = nibbles.len();
        let byte_count = (len + 1) / 2;
        let mut data = vec![0u8; byte_count];

        for (i, &nibble) in nibbles.iter().enumerate() {
            Self::set_nibble(&mut data, i, nibble);
        }

        (data, len)
    }

    /// Unpack nibbles from packed byte data
    fn unpack_nibbles(data: &[u8], len: usize) -> Vec<u8> {
        let mut nibbles = Vec::with_capacity(len);
        for i in 0..len {
            nibbles.push(Self::get_nibble(data, i));
        }
        nibbles
    }
}

impl Nibbles {
    /// Create `Nibbles` from hex-encoded nibbles (unpacked format)
    /// If the last nibble is 16, it's treated as the leaf flag
    pub fn from_hex(hex: Vec<u8>) -> Self {
        // Check if the last nibble is the leaf flag (16)
        let is_leaf = hex.last().map_or(false, |&n| n == 16);

        // Remove the leaf flag if present
        let nibbles = if is_leaf {
            &hex[..hex.len() - 1]
        } else {
            &hex[..]
        };

        // Pack the nibbles
        let (data, len) = Self::pack_nibbles(nibbles);

        Self {
            data,
            len,
            is_leaf,
            already_consumed: Vec::new(),
            consumed_len: 0,
        }
    }

    /// Splits incoming bytes into nibbles and appends the leaf flag (a 16 nibble at the end)
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_raw(bytes, true)
    }

    /// Create Nibbles from raw bytes. Each byte represents 2 nibbles.
    /// The `is_leaf` flag indicates whether this is a leaf node.
    pub fn from_raw(bytes: &[u8], is_leaf: bool) -> Self {
        // Each input byte becomes 2 nibbles, so the length is bytes.len() * 2
        let len = bytes.len() * 2;

        // Bytes are already in packed format (each byte = 2 nibbles)
        // Just copy them directly
        let data = bytes.to_vec();

        Self {
            data,
            len,
            is_leaf,
            already_consumed: Vec::new(),
            consumed_len: 0,
        }
    }

    /// Convert to unpacked nibble vector (for backward compatibility)
    pub fn into_vec(self) -> Vec<u8> {
        let mut nibbles = Self::unpack_nibbles(&self.data, self.len);
        if self.is_leaf {
            nibbles.push(16);
        }
        nibbles
    }

    /// Returns the amount of nibbles (including leaf flag if present)
    #[inline]
    pub fn len(&self) -> usize {
        self.len + if self.is_leaf { 1 } else { 0 }
    }

    /// Returns true if there are no nibbles (excluding leaf flag)
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0 && !self.is_leaf
    }

    /// If `prefix` is a prefix of self, move the offset after
    /// the prefix and return true, otherwise return false.
    pub fn skip_prefix(&mut self, prefix: &Nibbles) -> bool {
        // Check if self has at least as many nibbles as prefix
        if self.len < prefix.len {
            return false;
        }

        // Compare nibbles one by one
        for i in 0..prefix.len {
            if Self::get_nibble(&self.data, i) != Self::get_nibble(&prefix.data, i) {
                return false;
            }
        }

        // Prefix matches, so skip it
        // Add prefix nibbles to already_consumed
        for i in 0..prefix.len {
            let nibble = Self::get_nibble(&prefix.data, i);
            Self::set_nibble(&mut self.already_consumed, self.consumed_len, nibble);
            self.consumed_len += 1;
        }

        // Remove prefix from data by slicing
        let new_nibbles: Vec<u8> = (prefix.len..self.len)
            .map(|i| Self::get_nibble(&self.data, i))
            .collect();

        let (data, len) = Self::pack_nibbles(&new_nibbles);
        self.data = data;
        self.len = len;

        true
    }

    /// Compares self to another, comparing prefixes only in case of unequal lengths.
    pub fn compare_prefix(&self, prefix: &Nibbles) -> cmp::Ordering {
        let compare_len = self.len.min(prefix.len);

        for i in 0..compare_len {
            let self_nibble = Self::get_nibble(&self.data, i);
            let prefix_nibble = Self::get_nibble(&prefix.data, i);
            match self_nibble.cmp(&prefix_nibble) {
                cmp::Ordering::Equal => continue,
                other => return other,
            }
        }

        cmp::Ordering::Equal
    }

    /// Compares self to another and returns the shared nibble count (amount of nibbles that are equal, from the start)
    pub fn count_prefix(&self, other: &Nibbles) -> usize {
        let max_len = self.len.min(other.len);
        let mut count = 0;

        for i in 0..max_len {
            if Self::get_nibble(&self.data, i) == Self::get_nibble(&other.data, i) {
                count += 1;
            } else {
                break;
            }
        }

        count
    }

    /// Removes and returns the first nibble
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<u8> {
        if self.len == 0 {
            // If we have no nibbles left but have a leaf flag, return it
            if self.is_leaf {
                self.is_leaf = false;
                return Some(16);
            }
            return None;
        }

        // Get the first nibble
        let first_nibble = Self::get_nibble(&self.data, 0);

        // Add to consumed nibbles
        Self::set_nibble(&mut self.already_consumed, self.consumed_len, first_nibble);
        self.consumed_len += 1;

        // Shift all remaining nibbles left by one position
        for i in 0..self.len - 1 {
            let next_nibble = Self::get_nibble(&self.data, i + 1);
            Self::set_nibble(&mut self.data, i, next_nibble);
        }

        // Decrease length
        self.len -= 1;

        // If the length is now even and we had an odd length before, we can shrink the data vector
        if self.len % 2 == 0 && self.len > 0 {
            let byte_count = self.len / 2;
            self.data.truncate(byte_count);
        }

        Some(first_nibble)
    }

    /// Removes and returns the first nibble if it is a suitable choice index (aka < 16)
    pub fn next_choice(&mut self) -> Option<usize> {
        self.next().filter(|choice| *choice < 16).map(usize::from)
    }

    /// Returns the nibbles after the given offset
    pub fn offset(&self, offset: usize) -> Nibbles {
        let mut ret = self.slice(offset, self.len());

        // Update already_consumed to include the original already_consumed plus the skipped nibbles
        let mut new_consumed = Self::unpack_nibbles(&self.already_consumed, self.consumed_len);
        for i in 0..offset {
            if i < self.len {
                new_consumed.push(Self::get_nibble(&self.data, i));
            }
        }

        let (consumed_data, consumed_len) = Self::pack_nibbles(&new_consumed);
        ret.already_consumed = consumed_data;
        ret.consumed_len = consumed_len;

        ret
    }

    /// Returns the nibbles between the start and end indexes
    pub fn slice(&self, start: usize, end: usize) -> Nibbles {
        // Extract nibbles from start to end (excluding leaf flag)
        let actual_end = end.min(self.len);
        let actual_start = start.min(self.len);

        let mut nibbles = Vec::new();
        for i in actual_start..actual_end {
            nibbles.push(Self::get_nibble(&self.data, i));
        }

        // Check if we should include the leaf flag
        let include_leaf = end > self.len && self.is_leaf;

        let (data, len) = Self::pack_nibbles(&nibbles);

        Nibbles {
            data,
            len,
            is_leaf: include_leaf,
            already_consumed: Vec::new(),
            consumed_len: 0,
        }
    }

    /// Extends the nibbles with another list of nibbles
    pub fn extend(&mut self, other: &Nibbles) {
        // Append all nibbles from other to self
        for i in 0..other.len {
            let nibble = Self::get_nibble(&other.data, i);
            Self::set_nibble(&mut self.data, self.len, nibble);
            self.len += 1;
        }

        // If other has a leaf flag and we don't, inherit it
        if other.is_leaf && !self.is_leaf {
            self.is_leaf = true;
        }
    }

    /// Return the nibble at the given index, will panic if the index is out of range
    #[inline]
    pub fn at(&self, i: usize) -> usize {
        if i < self.len {
            Self::get_nibble(&self.data, i) as usize
        } else if i == self.len && self.is_leaf {
            16  // Return leaf flag if accessing the last position
        } else {
            panic!("Index {} out of range for Nibbles with length {}", i, self.len());
        }
    }

    /// Inserts a nibble at the start
    pub fn prepend(&mut self, nibble: u8) {
        // Shift all nibbles right by one position
        // First, ensure we have enough space
        if (self.len + 1 + 1) / 2 > self.data.len() {
            self.data.push(0);
        }

        // Shift from the end to the beginning
        for i in (0..self.len).rev() {
            let n = Self::get_nibble(&self.data, i);
            Self::set_nibble(&mut self.data, i + 1, n);
        }

        // Insert the new nibble at the start
        Self::set_nibble(&mut self.data, 0, nibble);
        self.len += 1;
    }

    /// Inserts a nibble at the end
    pub fn append(&mut self, nibble: u8) {
        // Append nibble at the end
        Self::set_nibble(&mut self.data, self.len, nibble);
        self.len += 1;
    }

    /// Taken from https://github.com/citahub/cita_trie/blob/master/src/nibbles.rs#L56
    /// Encodes the nibbles in compact form (Ethereum hex-prefix encoding)
    pub fn encode_compact(&self) -> Vec<u8> {
        let mut compact = vec![];
        let is_leaf = self.is_leaf;
        let nibble_count = self.len;  // Exclude leaf flag from encoding

        // node type    path length    |    prefix    hexchar
        // --------------------------------------------------
        // extension    even           |    0000      0x0
        // extension    odd            |    0001      0x1
        // leaf         even           |    0010      0x2
        // leaf         odd            |    0011      0x3

        // Determine if the path length is odd
        let is_odd = nibble_count % 2 == 1;

        // Calculate the prefix byte
        let mut prefix = if is_leaf { 0x20 } else { 0x00 };

        if is_odd {
            // If odd, include the first nibble in the prefix byte
            prefix += 0x10 + Self::get_nibble(&self.data, 0);
            compact.push(prefix);

            // Pack remaining nibbles (starting from index 1)
            for i in (1..nibble_count).step_by(2) {
                let high = Self::get_nibble(&self.data, i);
                let low = if i + 1 < nibble_count {
                    Self::get_nibble(&self.data, i + 1)
                } else {
                    0
                };
                compact.push((high << 4) | low);
            }
        } else {
            // If even, prefix byte is just the flags
            compact.push(prefix);

            // Pack all nibbles
            for i in (0..nibble_count).step_by(2) {
                let high = Self::get_nibble(&self.data, i);
                let low = if i + 1 < nibble_count {
                    Self::get_nibble(&self.data, i + 1)
                } else {
                    0
                };
                compact.push((high << 4) | low);
            }
        }

        compact
    }

    /// Encodes the nibbles in compact form
    pub fn decode_compact(compact: &[u8]) -> Self {
        Self::from_hex(compact_to_hex(compact))
    }

    /// Returns true if this represents a leaf node
    #[inline]
    pub fn is_leaf(&self) -> bool {
        self.is_leaf
    }

    /// Combines the nibbles into bytes (leaf flag is excluded)
    pub fn to_bytes(&self) -> Vec<u8> {
        // Data is already packed
        // If we have an odd number of nibbles, the last byte only uses the high nibble
        // For to_bytes, we should only return complete bytes
        let complete_byte_count = self.len / 2;

        // If we have an odd number of nibbles, we need to handle the last nibble specially
        if self.len % 2 == 1 {
            // Include the byte with the odd nibble, but it only represents half a byte worth of data
            self.data[..complete_byte_count + 1].to_vec()
        } else {
            // All nibbles are in complete bytes
            self.data[..complete_byte_count].to_vec()
        }
    }

    /// Concatenates self and another Nibbles returning a new Nibbles
    pub fn concat(&self, other: &Nibbles) -> Nibbles {
        let mut result = self.clone();
        result.extend(other);
        result
    }

    /// Returns a copy of self with the nibble added at the end
    pub fn append_new(&self, nibble: u8) -> Nibbles {
        let mut result = self.clone();
        result.append(nibble);
        result
    }

    /// Return already consumed parts of path
    pub fn current(&self) -> Nibbles {
        Nibbles {
            data: self.already_consumed.clone(),
            len: self.consumed_len,
            is_leaf: false,
            already_consumed: Vec::new(),
            consumed_len: 0,
        }
    }

    /// Empties `self.data` and returns the content
    pub fn take(&mut self) -> Self {
        Nibbles {
            data: mem::take(&mut self.data),
            len: mem::replace(&mut self.len, 0),
            is_leaf: mem::replace(&mut self.is_leaf, false),
            already_consumed: mem::take(&mut self.already_consumed),
            consumed_len: mem::replace(&mut self.consumed_len, 0),
        }
    }
}

impl AsRef<[u8]> for Nibbles {
    /// Returns a reference to the packed nibble data
    /// Note: This returns packed data (2 nibbles per byte), not unpacked nibbles
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl RLPEncode for Nibbles {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.data)
            .encode_field(&self.len)
            .encode_field(&self.is_leaf)
            .finish();
    }
}

impl RLPDecode for Nibbles {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (data, decoder) = decoder.decode_field("data")?;
        let (len, decoder) = decoder.decode_field("len")?;
        let (is_leaf, decoder) = decoder.decode_field("is_leaf")?;
        Ok((
            Self {
                data,
                len,
                is_leaf,
                already_consumed: Vec::new(),
                consumed_len: 0,
            },
            decoder.finish()?,
        ))
    }
}

// Code taken from https://github.com/ethereum/go-ethereum/blob/a1093d98eb3260f2abf340903c2d968b2b891c11/trie/encoding.go#L82
fn compact_to_hex(compact: &[u8]) -> Vec<u8> {
    if compact.is_empty() {
        return vec![];
    }
    let mut base = keybytes_to_hex(compact);
    // delete terminator flag
    if base[0] < 2 {
        base = base[..base.len() - 1].to_vec();
    }
    // apply odd flag
    let chop = 2 - (base[0] & 1) as usize;
    base[chop..].to_vec()
}

// Code taken from https://github.com/ethereum/go-ethereum/blob/a1093d98eb3260f2abf340903c2d968b2b891c11/trie/encoding.go#L96
fn keybytes_to_hex(keybytes: &[u8]) -> Vec<u8> {
    let l = keybytes.len() * 2 + 1;
    let mut nibbles = vec![0; l];
    for (i, b) in keybytes.iter().enumerate() {
        nibbles[i * 2] = b / 16;
        nibbles[i * 2 + 1] = b % 16;
    }
    nibbles[l - 1] = 16;
    nibbles
}

#[cfg(test)]
mod test {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn skip_prefix_true() {
        let mut a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3]);
        assert!(a.skip_prefix(&b));
        assert_eq!(a.into_vec(), vec![4, 5])
    }

    #[test]
    fn skip_prefix_true_same_length() {
        let mut a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert!(a.skip_prefix(&b));
        assert!(a.is_empty());
    }

    #[test]
    fn skip_prefix_longer_prefix() {
        let mut a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert!(!a.skip_prefix(&b));
        assert_eq!(a.into_vec(), vec![1, 2, 3])
    }

    #[test]
    fn skip_prefix_false() {
        let mut a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 4]);
        assert!(!a.skip_prefix(&b));
        assert_eq!(a.into_vec(), vec![1, 2, 3, 4, 5])
    }

    #[test]
    fn count_prefix_all() {
        let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert_eq!(a.count_prefix(&b), a.len());
    }

    #[test]
    fn count_prefix_partial() {
        let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3]);
        assert_eq!(a.count_prefix(&b), b.len());
    }

    #[test]
    fn count_prefix_none() {
        let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![2, 3, 4, 5, 6]);
        assert_eq!(a.count_prefix(&b), 0);
    }

    #[test]
    fn compare_prefix_equal() {
        let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Equal);
    }

    #[test]
    fn compare_prefix_less() {
        let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 4, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Less);
    }

    #[test]
    fn compare_prefix_greater() {
        let a = Nibbles::from_hex(vec![1, 2, 4, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Greater);
    }

    #[test]
    fn compare_prefix_equal_b_longer() {
        let a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Equal);
    }

    #[test]
    fn compare_prefix_equal_a_longer() {
        let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3]);
        assert_eq!(a.compare_prefix(&b), Ordering::Equal);
    }
}
