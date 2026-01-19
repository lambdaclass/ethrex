use std::cmp;

use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

/// Special nibble value indicating a leaf node terminator
pub const LEAF_FLAG: u8 = 16;

/// Maximum packed bytes for nibble data.
/// For Ethereum: account paths are 64 nibbles (32 bytes), storage with prefix ~130 nibbles (65 bytes).
/// Tests may use longer paths, so we set this to 100 to accommodate test scenarios.
const MAX_PACKED_BYTES: usize = 100;

/// Inline byte buffer with fixed capacity for stack allocation
#[derive(Clone, Copy)]
struct InlineBytes {
    data: [u8; MAX_PACKED_BYTES],
    len: u8,
}

impl Default for InlineBytes {
    #[inline]
    fn default() -> Self {
        Self {
            data: [0u8; MAX_PACKED_BYTES],
            len: 0,
        }
    }
}

impl InlineBytes {
    #[inline]
    fn new() -> Self {
        Self::default()
    }

    #[inline]
    fn from_slice(slice: &[u8]) -> Self {
        debug_assert!(slice.len() <= MAX_PACKED_BYTES);
        let mut data = [0u8; MAX_PACKED_BYTES];
        let len = slice.len().min(MAX_PACKED_BYTES);
        data[..len].copy_from_slice(&slice[..len]);
        Self { data, len: len as u8 }
    }

    #[inline]
    fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    #[allow(dead_code)]
    fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    fn as_slice(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }

    #[inline]
    fn clear(&mut self) {
        self.len = 0;
    }

    #[inline]
    fn push(&mut self, byte: u8) {
        debug_assert!((self.len as usize) < MAX_PACKED_BYTES);
        self.data[self.len as usize] = byte;
        self.len += 1;
    }

    #[inline]
    fn get(&self, index: usize) -> u8 {
        debug_assert!(index < self.len as usize);
        self.data[index]
    }

    #[inline]
    fn get_mut(&mut self, index: usize) -> &mut u8 {
        debug_assert!(index < self.len as usize);
        &mut self.data[index]
    }

    #[inline]
    fn set_len(&mut self, len: usize) {
        debug_assert!(len <= MAX_PACKED_BYTES);
        self.len = len as u8;
    }

    #[inline]
    fn reserve(&mut self, additional: usize) {
        // No-op for fixed-size buffer, but check capacity
        debug_assert!(self.len as usize + additional <= MAX_PACKED_BYTES);
    }

    #[inline]
    fn drain_front(&mut self, count: usize) {
        if count >= self.len as usize {
            self.len = 0;
        } else {
            let new_len = self.len as usize - count;
            self.data.copy_within(count..self.len as usize, 0);
            self.len = new_len as u8;
        }
    }

    #[allow(dead_code)]
    fn to_vec(&self) -> Vec<u8> {
        self.as_slice().to_vec()
    }
}

// Implement serde for InlineBytes
impl serde::Serialize for InlineBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.as_slice().serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for InlineBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let vec = Vec::<u8>::deserialize(deserializer)?;
        Ok(Self::from_slice(&vec))
    }
}

// Implement rkyv for InlineBytes
impl rkyv::Archive for InlineBytes {
    type Archived = rkyv::vec::ArchivedVec<u8>;
    type Resolver = rkyv::vec::VecResolver;

    fn resolve(&self, resolver: Self::Resolver, out: rkyv::Place<Self::Archived>) {
        rkyv::vec::ArchivedVec::resolve_from_slice(self.as_slice(), resolver, out);
    }
}

impl<S: rkyv::ser::Allocator + rkyv::ser::Writer + rkyv::rancor::Fallible + ?Sized>
    rkyv::Serialize<S> for InlineBytes
{
    fn serialize(&self, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        rkyv::vec::ArchivedVec::serialize_from_slice(self.as_slice(), serializer)
    }
}

impl<D: rkyv::rancor::Fallible + ?Sized> rkyv::Deserialize<InlineBytes, D>
    for rkyv::vec::ArchivedVec<u8>
{
    fn deserialize(&self, _deserializer: &mut D) -> Result<InlineBytes, D::Error> {
        Ok(InlineBytes::from_slice(self.as_slice()))
    }
}

/// Packed nibbles representation storing 2 nibbles per byte.
///
/// Nibbles are stored packed with the high nibble first:
/// - Byte 0: nibble[0] << 4 | nibble[1]
/// - Byte 1: nibble[2] << 4 | nibble[3]
/// - etc.
///
/// For odd-length nibbles, the last byte's low nibble is 0 (padding).
///
/// The leaf flag (16) is stored as a separate boolean since it doesn't fit in 4 bits.
/// When `has_leaf_flag` is true, `len()` includes the leaf flag in the count.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Nibbles {
    /// Packed nibble data (2 nibbles per byte, values 0-15 only)
    data: InlineBytes,
    /// Number of nibbles (NOT including the leaf flag)
    nibble_count: u16,
    /// Whether this path ends with a leaf flag (16)
    has_leaf_flag: bool,
    /// Packed consumed nibbles (for path tracking via `current()`)
    consumed_data: InlineBytes,
    /// Number of consumed nibbles (NOT including leaf flag)
    consumed_count: u16,
    /// Whether the consumed path had a leaf flag
    consumed_has_leaf: bool,
}

impl std::fmt::Debug for Nibbles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Nibbles([")?;
        for i in 0..self.len() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", self.get_nibble(i))?;
        }
        write!(f, "])")
    }
}

impl Default for Nibbles {
    fn default() -> Self {
        Self {
            data: InlineBytes::new(),
            nibble_count: 0,
            has_leaf_flag: false,
            consumed_data: InlineBytes::new(),
            consumed_count: 0,
            consumed_has_leaf: false,
        }
    }
}

// Custom impls to ignore the `consumed` fields for equality/ordering/hashing

impl PartialEq for Nibbles {
    fn eq(&self, other: &Nibbles) -> bool {
        if self.nibble_count != other.nibble_count || self.has_leaf_flag != other.has_leaf_flag {
            return false;
        }
        // Compare only the relevant bytes
        let byte_len = (self.nibble_count as usize + 1) / 2;
        if byte_len == 0 {
            return true;
        }
        // If odd length, mask the last byte's low nibble
        if self.nibble_count % 2 == 1 {
            let last_idx = byte_len - 1;
            self.data.as_slice()[..last_idx] == other.data.as_slice()[..last_idx]
                && (self.data.get(last_idx) & 0xF0) == (other.data.get(last_idx) & 0xF0)
        } else {
            self.data.as_slice()[..byte_len] == other.data.as_slice()[..byte_len]
        }
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
        let min_len = self.len().min(other.len());
        for i in 0..min_len {
            match self.get_nibble(i).cmp(&other.get_nibble(i)) {
                cmp::Ordering::Equal => continue,
                ord => return ord,
            }
        }
        self.len().cmp(&other.len())
    }
}

impl std::hash::Hash for Nibbles {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.nibble_count.hash(state);
        self.has_leaf_flag.hash(state);
        let byte_len = (self.nibble_count as usize + 1) / 2;
        if byte_len > 0 {
            if self.nibble_count % 2 == 0 {
                self.data.as_slice()[..byte_len].hash(state);
            } else {
                if byte_len > 1 {
                    self.data.as_slice()[..byte_len - 1].hash(state);
                }
                (self.data.get(byte_len - 1) & 0xF0).hash(state);
            }
        }
    }
}

impl Nibbles {
    /// Creates empty nibbles
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates nibbles from unpacked hex representation (1 nibble per byte)
    /// Values should be 0-15, or 16 for leaf flag
    pub fn from_hex(hex: Vec<u8>) -> Self {
        let has_leaf = hex.last() == Some(&LEAF_FLAG);
        let nibble_count = if has_leaf { hex.len() - 1 } else { hex.len() };

        // Pack nibbles: 2 per byte
        let byte_len = (nibble_count + 1) / 2;
        let mut data = InlineBytes::new();
        data.set_len(byte_len);

        for i in 0..nibble_count {
            let nibble = hex[i];
            debug_assert!(nibble < 16, "Invalid nibble value: {}", nibble);
            let byte_idx = i / 2;
            if i % 2 == 0 {
                *data.get_mut(byte_idx) = nibble << 4;
            } else {
                *data.get_mut(byte_idx) |= nibble;
            }
        }

        Self {
            data,
            nibble_count: nibble_count as u16,
            has_leaf_flag: has_leaf,
            consumed_data: InlineBytes::new(),
            consumed_count: 0,
            consumed_has_leaf: false,
        }
    }

    /// Creates nibbles from a byte slice (each byte becomes 2 nibbles)
    /// Assumes this is a leaf path (adds leaf flag)
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_raw(bytes, true)
    }

    /// Splits incoming bytes into nibbles, optionally appending the leaf flag
    pub fn from_raw(bytes: &[u8], is_leaf: bool) -> Self {
        Self {
            data: InlineBytes::from_slice(bytes), // Bytes are already packed: each byte is 2 nibbles
            nibble_count: (bytes.len() * 2) as u16,
            has_leaf_flag: is_leaf,
            consumed_data: InlineBytes::new(),
            consumed_count: 0,
            consumed_has_leaf: false,
        }
    }

    /// Returns the nibbles as unpacked hex (1 nibble per byte), including leaf flag if present
    pub fn to_hex(&self) -> Vec<u8> {
        let total_len = self.len();
        let mut result = Vec::with_capacity(total_len);
        for i in 0..self.nibble_count as usize {
            result.push(self.get_raw_nibble(i));
        }
        if self.has_leaf_flag {
            result.push(LEAF_FLAG);
        }
        result
    }

    /// Converts nibbles to owned Vec (unpacked format for compatibility)
    pub fn into_vec(self) -> Vec<u8> {
        self.to_hex()
    }

    /// Returns the total number of nibbles (including leaf flag if present)
    #[inline]
    pub fn len(&self) -> usize {
        self.nibble_count as usize + if self.has_leaf_flag { 1 } else { 0 }
    }

    /// Returns true if there are no nibbles
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nibble_count == 0 && !self.has_leaf_flag
    }

    /// Gets the nibble at the given index (0-indexed).
    /// If index equals nibble_count and has_leaf_flag is true, returns LEAF_FLAG.
    #[inline]
    pub fn get_nibble(&self, index: usize) -> u8 {
        if index == self.nibble_count as usize && self.has_leaf_flag {
            return LEAF_FLAG;
        }
        self.get_raw_nibble(index)
    }

    /// Gets a raw nibble (not the leaf flag) at the given index
    #[inline]
    fn get_raw_nibble(&self, index: usize) -> u8 {
        debug_assert!(index < self.nibble_count as usize);
        let byte_idx = index / 2;
        let byte = self.data.get(byte_idx);
        if index % 2 == 0 {
            byte >> 4
        } else {
            byte & 0x0F
        }
    }

    /// Return the nibble at the given index as usize
    #[inline]
    pub fn at(&self, i: usize) -> usize {
        self.get_nibble(i) as usize
    }

    /// Returns true if the nibbles represent a leaf (has terminator)
    #[inline]
    pub fn is_leaf(&self) -> bool {
        self.has_leaf_flag
    }

    /// Removes nibbles from the start
    fn remove_prefix(&mut self, count: usize, also_remove_leaf: bool) {
        if count == 0 && !also_remove_leaf {
            return;
        }
        if count >= self.nibble_count as usize {
            self.data.clear();
            self.nibble_count = 0;
            if also_remove_leaf {
                self.has_leaf_flag = false;
            }
            return;
        }

        let new_count = self.nibble_count as usize - count;

        // Shift nibbles left by `count` positions
        if count % 2 == 0 {
            // Even shift: just remove bytes from the front
            let bytes_to_remove = count / 2;
            self.data.drain_front(bytes_to_remove);
        } else {
            // Odd shift: need to re-pack
            let new_byte_len = (new_count + 1) / 2;
            let mut new_data = InlineBytes::new();
            new_data.set_len(new_byte_len);
            for i in 0..new_count {
                let nibble = self.get_raw_nibble(count + i);
                let byte_idx = i / 2;
                if i % 2 == 0 {
                    *new_data.get_mut(byte_idx) = nibble << 4;
                } else {
                    *new_data.get_mut(byte_idx) |= nibble;
                }
            }
            self.data = new_data;
        }
        self.nibble_count = new_count as u16;
    }

    /// Compares this path with a prefix, returning ordering
    pub fn compare_prefix(&self, prefix: &Nibbles) -> cmp::Ordering {
        let cmp_len = self.len().min(prefix.len());
        for i in 0..cmp_len {
            match self.get_nibble(i).cmp(&prefix.get_nibble(i)) {
                cmp::Ordering::Equal => continue,
                ord => return ord,
            }
        }
        cmp::Ordering::Equal
    }

    /// Returns the number of matching nibbles from the start
    pub fn count_prefix(&self, other: &Nibbles) -> usize {
        let min_len = self.len().min(other.len());
        for i in 0..min_len {
            if self.get_nibble(i) != other.get_nibble(i) {
                return i;
            }
        }
        min_len
    }

    /// Removes and returns the first nibble
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }

        // Special case: only the leaf flag remains
        if self.nibble_count == 0 && self.has_leaf_flag {
            self.consumed_has_leaf = true;
            self.has_leaf_flag = false;
            return Some(LEAF_FLAG);
        }

        let first = self.get_raw_nibble(0);
        self.append_consumed_nibble(first);
        self.remove_prefix(1, false);
        Some(first)
    }

    /// Removes and returns the first nibble if it's a valid choice index (< 16)
    pub fn next_choice(&mut self) -> Option<usize> {
        self.next().filter(|&n| n < LEAF_FLAG).map(usize::from)
    }

    /// Returns nibbles after the given offset
    pub fn offset(&self, offset: usize) -> Nibbles {
        let total_len = self.len();
        if offset >= total_len {
            // Build consumed_data from self.consumed_data + all of self
            let mut result = Nibbles {
                data: InlineBytes::new(),
                nibble_count: 0,
                has_leaf_flag: false,
                consumed_data: self.consumed_data,
                consumed_count: self.consumed_count,
                consumed_has_leaf: self.consumed_has_leaf || self.has_leaf_flag,
            };
            for i in 0..self.nibble_count as usize {
                result.append_consumed_nibble(self.get_raw_nibble(i));
            }
            return result;
        }

        // Build consumed from self.consumed_data + first `offset` nibbles
        let mut consumed_data = self.consumed_data;
        let mut consumed_count = self.consumed_count;
        let mut consumed_has_leaf = self.consumed_has_leaf;

        for i in 0..offset {
            if i == self.nibble_count as usize && self.has_leaf_flag {
                consumed_has_leaf = true;
            } else {
                let nibble = self.get_raw_nibble(i);
                let idx = consumed_count as usize;
                consumed_count += 1;
                if idx % 2 == 0 {
                    consumed_data.reserve(1);
                    if consumed_data.len() <= idx / 2 {
                        consumed_data.push(nibble << 4);
                    } else {
                        *consumed_data.get_mut(idx / 2) = nibble << 4;
                    }
                } else {
                    *consumed_data.get_mut(idx / 2) |= nibble;
                }
            }
        }

        // Build remaining data
        let remaining_nibbles = self.nibble_count as usize - offset;
        let new_byte_len = (remaining_nibbles + 1) / 2;
        let mut new_data = InlineBytes::new();
        new_data.set_len(new_byte_len);

        for i in 0..remaining_nibbles {
            let nibble = self.get_raw_nibble(offset + i);
            let byte_idx = i / 2;
            if i % 2 == 0 {
                *new_data.get_mut(byte_idx) = nibble << 4;
            } else {
                *new_data.get_mut(byte_idx) |= nibble;
            }
        }

        Nibbles {
            data: new_data,
            nibble_count: remaining_nibbles as u16,
            has_leaf_flag: self.has_leaf_flag,
            consumed_data,
            consumed_count,
            consumed_has_leaf,
        }
    }

    /// Returns a slice of nibbles from start to end (exclusive)
    pub fn slice(&self, start: usize, end: usize) -> Nibbles {
        debug_assert!(start <= end);
        debug_assert!(end <= self.len());

        let slice_len = end - start;
        if slice_len == 0 {
            return Nibbles::default();
        }

        // Check if slice includes the leaf flag
        let includes_leaf = self.has_leaf_flag && end == self.len();
        let nibble_count = if includes_leaf { slice_len - 1 } else { slice_len };

        let new_byte_len = (nibble_count + 1) / 2;
        let mut new_data = InlineBytes::new();
        new_data.set_len(new_byte_len);

        for i in 0..nibble_count {
            let nibble = self.get_raw_nibble(start + i);
            let byte_idx = i / 2;
            if i % 2 == 0 {
                *new_data.get_mut(byte_idx) = nibble << 4;
            } else {
                *new_data.get_mut(byte_idx) |= nibble;
            }
        }

        Nibbles {
            data: new_data,
            nibble_count: nibble_count as u16,
            has_leaf_flag: includes_leaf,
            consumed_data: InlineBytes::new(),
            consumed_count: 0,
            consumed_has_leaf: false,
        }
    }

    /// Extends self with nibbles from another Nibbles (modifies in place)
    /// If `other` has a leaf flag, it will be copied to self.
    pub fn extend(&mut self, other: &Nibbles) {
        if other.is_empty() {
            return;
        }

        let old_count = self.nibble_count as usize;
        let new_count = old_count + other.nibble_count as usize;
        let new_byte_len = (new_count + 1) / 2;

        self.data.reserve(new_byte_len.saturating_sub(self.data.len()));
        while self.data.len() < new_byte_len {
            self.data.push(0);
        }

        // Copy nibbles from other
        for i in 0..other.nibble_count as usize {
            let nibble = other.get_raw_nibble(i);
            let idx = old_count + i;
            if idx % 2 == 0 {
                *self.data.get_mut(idx / 2) = nibble << 4;
            } else {
                *self.data.get_mut(idx / 2) |= nibble;
            }
        }

        self.nibble_count = new_count as u16;
        // Copy the leaf flag from other if it has one
        if other.has_leaf_flag {
            self.has_leaf_flag = true;
        }
    }

    /// Appends a single nibble (modifies in place)
    pub fn append(&mut self, nibble: u8) {
        if nibble == LEAF_FLAG {
            self.has_leaf_flag = true;
            return;
        }

        let idx = self.nibble_count as usize;
        self.nibble_count += 1;

        if idx % 2 == 0 {
            self.data.push(nibble << 4);
        } else {
            let byte_idx = idx / 2;
            *self.data.get_mut(byte_idx) |= nibble;
        }
    }

    /// Encodes nibbles in compact form (for RLP encoding in trie nodes)
    pub fn encode_compact(&self) -> Vec<u8> {
        let is_leaf = self.has_leaf_flag;
        let len = self.nibble_count as usize;

        if len == 0 {
            return vec![if is_leaf { 0x20 } else { 0x00 }];
        }

        let odd = len % 2 == 1;
        let first_byte = match (is_leaf, odd) {
            (false, false) => 0x00,
            (false, true) => 0x10 | self.get_raw_nibble(0),
            (true, false) => 0x20,
            (true, true) => 0x30 | self.get_raw_nibble(0),
        };

        let start_idx = if odd { 1 } else { 0 };
        let remaining = len - start_idx;
        let mut result = Vec::with_capacity(1 + (remaining + 1) / 2);
        result.push(first_byte);

        // Pack remaining nibbles
        let mut i = start_idx;
        while i < len {
            let high = self.get_raw_nibble(i);
            let low = if i + 1 < len {
                self.get_raw_nibble(i + 1)
            } else {
                0
            };
            result.push((high << 4) | low);
            i += 2;
        }

        result
    }

    /// Decodes nibbles from compact form
    pub fn decode_compact(compact: &[u8]) -> Self {
        if compact.is_empty() {
            return Self::default();
        }

        let first = compact[0];
        let is_leaf = (first & 0x20) != 0;
        let odd = (first & 0x10) != 0;

        let mut nibbles = Vec::new();

        if odd {
            nibbles.push(first & 0x0F);
        }

        for &byte in &compact[1..] {
            nibbles.push(byte >> 4);
            nibbles.push(byte & 0x0F);
        }

        // Remove trailing zero if we have even length from odd encoding
        if !odd && nibbles.len() % 2 == 1 {
            nibbles.pop();
        }

        let nibble_count = nibbles.len();
        let byte_len = (nibble_count + 1) / 2;
        let mut data = InlineBytes::new();
        data.set_len(byte_len);

        for i in 0..nibble_count {
            let nibble = nibbles[i];
            let byte_idx = i / 2;
            if i % 2 == 0 {
                *data.get_mut(byte_idx) = nibble << 4;
            } else {
                *data.get_mut(byte_idx) |= nibble;
            }
        }

        Self {
            data,
            nibble_count: nibble_count as u16,
            has_leaf_flag: is_leaf,
            consumed_data: InlineBytes::new(),
            consumed_count: 0,
            consumed_has_leaf: false,
        }
    }

    /// Converts nibbles to bytes (packs 2 nibbles per byte)
    pub fn to_bytes(&self) -> Vec<u8> {
        // The data is already in packed format (2 nibbles per byte)
        // Just need to return the right number of bytes
        let byte_count = (self.nibble_count as usize + 1) / 2;

        if self.nibble_count % 2 == 0 {
            // Even number of nibbles, return as-is
            self.data.as_slice()[..byte_count].to_vec()
        } else {
            // Odd number of nibbles, last byte's low nibble is padding
            let mut result = self.data.as_slice()[..byte_count].to_vec();
            if !result.is_empty() {
                result[byte_count - 1] &= 0xF0; // Clear low nibble
            }
            result
        }
    }

    /// Concatenates self and another Nibbles, returning a new Nibbles
    pub fn concat(&self, other: &Nibbles) -> Nibbles {
        if self.is_empty() {
            return Nibbles {
                data: other.data,
                nibble_count: other.nibble_count,
                has_leaf_flag: other.has_leaf_flag,
                consumed_data: self.consumed_data,
                consumed_count: self.consumed_count,
                consumed_has_leaf: self.consumed_has_leaf,
            };
        }
        if other.is_empty() {
            return *self;
        }

        let total_nibbles = self.nibble_count as usize + other.nibble_count as usize;
        let new_byte_len = (total_nibbles + 1) / 2;
        let mut new_data = InlineBytes::new();
        new_data.set_len(new_byte_len);

        // Copy self's nibbles
        for i in 0..self.nibble_count as usize {
            let nibble = self.get_raw_nibble(i);
            let byte_idx = i / 2;
            if i % 2 == 0 {
                *new_data.get_mut(byte_idx) = nibble << 4;
            } else {
                *new_data.get_mut(byte_idx) |= nibble;
            }
        }

        // Copy other's nibbles
        for i in 0..other.nibble_count as usize {
            let nibble = other.get_raw_nibble(i);
            let idx = self.nibble_count as usize + i;
            let byte_idx = idx / 2;
            if idx % 2 == 0 {
                *new_data.get_mut(byte_idx) = nibble << 4;
            } else {
                *new_data.get_mut(byte_idx) |= nibble;
            }
        }

        Nibbles {
            data: new_data,
            nibble_count: total_nibbles as u16,
            has_leaf_flag: other.has_leaf_flag, // Use other's leaf flag
            consumed_data: self.consumed_data,
            consumed_count: self.consumed_count,
            consumed_has_leaf: self.consumed_has_leaf,
        }
    }

    /// Prepends a single nibble (modifies in place)
    pub fn prepend_nibble(&mut self, nibble: u8) {
        debug_assert!(nibble < 16, "Invalid nibble value: {}", nibble);

        let new_count = 1 + self.nibble_count as usize;
        let new_byte_len = (new_count + 1) / 2;
        let mut new_data = InlineBytes::new();
        new_data.set_len(new_byte_len);

        // Set the first nibble
        *new_data.get_mut(0) = nibble << 4;

        // Copy self's nibbles starting at index 1
        for i in 0..self.nibble_count as usize {
            let n = self.get_raw_nibble(i);
            let idx = 1 + i;
            let byte_idx = idx / 2;
            if idx % 2 == 0 {
                *new_data.get_mut(byte_idx) = n << 4;
            } else {
                *new_data.get_mut(byte_idx) |= n;
            }
        }

        self.data = new_data;
        self.nibble_count = new_count as u16;
    }

    /// Prepends nibbles from another Nibbles (modifies in place)
    pub fn prepend(&mut self, other: &Nibbles) {
        if other.is_empty() {
            return;
        }

        let new_count = other.nibble_count as usize + self.nibble_count as usize;
        let new_byte_len = (new_count + 1) / 2;
        let mut new_data = InlineBytes::new();
        new_data.set_len(new_byte_len);

        // Copy other's nibbles first
        for i in 0..other.nibble_count as usize {
            let nibble = other.get_raw_nibble(i);
            let byte_idx = i / 2;
            if i % 2 == 0 {
                *new_data.get_mut(byte_idx) = nibble << 4;
            } else {
                *new_data.get_mut(byte_idx) |= nibble;
            }
        }

        // Copy self's nibbles
        for i in 0..self.nibble_count as usize {
            let nibble = self.get_raw_nibble(i);
            let idx = other.nibble_count as usize + i;
            let byte_idx = idx / 2;
            if idx % 2 == 0 {
                *new_data.get_mut(byte_idx) = nibble << 4;
            } else {
                *new_data.get_mut(byte_idx) |= nibble;
            }
        }

        self.data = new_data;
        self.nibble_count = new_count as u16;
        // Keep self's leaf flag
    }

    /// Checks if `prefix` is a prefix of self, and if so, consumes it
    /// Returns true if prefix was found and consumed
    pub fn skip_prefix(&mut self, prefix: &Nibbles) -> bool {
        if prefix.len() > self.len() {
            return false;
        }

        // Check if prefix matches
        for i in 0..prefix.nibble_count as usize {
            if self.get_nibble(i) != prefix.get_nibble(i) {
                return false;
            }
        }

        // Check leaf flag if prefix has one
        if prefix.has_leaf_flag {
            if !self.has_leaf_flag {
                return false;
            }
            // Both have leaf flag and prefix nibbles match
        }

        // Consume the prefix
        for i in 0..prefix.nibble_count as usize {
            self.append_consumed_nibble(self.get_raw_nibble(i));
        }
        self.remove_prefix(prefix.nibble_count as usize, prefix.has_leaf_flag);
        if prefix.has_leaf_flag {
            self.consumed_has_leaf = true;
        }

        true
    }

    /// Appends a nibble to the consumed path
    fn append_consumed_nibble(&mut self, nibble: u8) {
        let idx = self.consumed_count as usize;
        self.consumed_count += 1;
        if idx % 2 == 0 {
            self.consumed_data.push(nibble << 4);
        } else {
            let byte_idx = idx / 2;
            *self.consumed_data.get_mut(byte_idx) |= nibble;
        }
    }

    /// Creates a new Nibbles with a single nibble appended
    pub fn append_new(&self, nibble: u8) -> Nibbles {
        let mut result = *self;
        result.append(nibble);
        result
    }

    /// Returns the already consumed path (for error reporting)
    pub fn current(&self) -> Nibbles {
        Nibbles {
            data: self.consumed_data,
            nibble_count: self.consumed_count,
            has_leaf_flag: self.consumed_has_leaf,
            consumed_data: InlineBytes::new(),
            consumed_count: 0,
            consumed_has_leaf: false,
        }
    }

    /// Empties self and returns the contents
    pub fn take(&mut self) -> Self {
        let result = *self;
        *self = Self::default();
        result
    }
}

impl AsRef<[u8]> for Nibbles {
    /// Returns a reference to the packed byte data.
    /// Note: In packed representation, 2 nibbles are stored per byte.
    /// For unpacked nibbles, use `to_hex()`.
    fn as_ref(&self) -> &[u8] {
        self.data.as_slice()
    }
}

impl Nibbles {
    /// Returns unpacked nibbles as a Vec (for compatibility with old API)
    pub fn as_slice(&self) -> Vec<u8> {
        self.to_hex()
    }
}

impl RLPEncode for Nibbles {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let hex = self.to_hex();
        Encoder::new(buf).encode_field(&hex).finish();
    }
}

impl RLPDecode for Nibbles {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (data, decoder): (Vec<u8>, _) = decoder.decode_field("data")?;
        Ok((Self::from_hex(data), decoder.finish()?))
    }
}

#[cfg(test)]
pub mod test {
    use super::*;

    #[test]
    fn from_hex_basic() {
        let n = Nibbles::from_hex(vec![1, 2, 3, 4, 16]);
        assert_eq!(n.len(), 5);
        assert_eq!(n.nibble_count, 4);
        assert!(n.has_leaf_flag);
        assert_eq!(n.get_nibble(0), 1);
        assert_eq!(n.get_nibble(1), 2);
        assert_eq!(n.get_nibble(2), 3);
        assert_eq!(n.get_nibble(3), 4);
        assert_eq!(n.get_nibble(4), LEAF_FLAG);
    }

    #[test]
    fn from_bytes_basic() {
        let n = Nibbles::from_bytes(&[0x12, 0x34]);
        assert_eq!(n.len(), 5); // 4 nibbles + leaf flag
        assert_eq!(n.to_hex(), vec![1, 2, 3, 4, 16]);
    }

    #[test]
    fn from_bytes_empty() {
        let n = Nibbles::from_bytes(&[]);
        assert_eq!(n.len(), 1); // Just leaf flag
        assert!(n.has_leaf_flag);
    }

    #[test]
    fn to_hex_roundtrip() {
        let original = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let n = Nibbles::from_hex(original.clone());
        assert_eq!(n.to_hex(), original);
    }

    #[test]
    fn to_bytes_basic() {
        let n = Nibbles::from_hex(vec![1, 2, 3, 4]);
        assert_eq!(n.to_bytes(), vec![0x12, 0x34]);
    }

    #[test]
    fn encode_compact_even_leaf() {
        let n = Nibbles::from_hex(vec![1, 2, 3, 4, 16]);
        assert_eq!(n.encode_compact(), vec![0x20, 0x12, 0x34]);
    }

    #[test]
    fn encode_compact_odd_leaf() {
        let n = Nibbles::from_hex(vec![1, 2, 3, 16]);
        assert_eq!(n.encode_compact(), vec![0x31, 0x23]);
    }

    #[test]
    fn encode_compact_even_extension() {
        let n = Nibbles::from_hex(vec![1, 2, 3, 4]);
        assert_eq!(n.encode_compact(), vec![0x00, 0x12, 0x34]);
    }

    #[test]
    fn encode_compact_odd_extension() {
        let n = Nibbles::from_hex(vec![1, 2, 3]);
        assert_eq!(n.encode_compact(), vec![0x11, 0x23]);
    }

    #[test]
    fn decode_compact_roundtrip() {
        let cases = vec![
            vec![1, 2, 3, 4, 16],    // even leaf
            vec![1, 2, 3, 16],       // odd leaf
            vec![1, 2, 3, 4],        // even extension
            vec![1, 2, 3],           // odd extension
            vec![16],                // just leaf
            vec![],                  // empty
        ];
        for original in cases {
            let n = Nibbles::from_hex(original.clone());
            let compact = n.encode_compact();
            let decoded = Nibbles::decode_compact(&compact);
            assert_eq!(decoded.to_hex(), original, "Failed for {:?}", original);
        }
    }

    #[test]
    fn skip_prefix_true() {
        let mut n = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let prefix = Nibbles::from_hex(vec![1, 2]);
        assert!(n.skip_prefix(&prefix));
        assert_eq!(n.to_hex(), vec![3, 4, 5]);
    }

    #[test]
    fn skip_prefix_false() {
        let mut n = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let prefix = Nibbles::from_hex(vec![1, 3]);
        assert!(!n.skip_prefix(&prefix));
        assert_eq!(n.to_hex(), vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn skip_prefix_longer_prefix() {
        let mut n = Nibbles::from_hex(vec![1, 2]);
        let prefix = Nibbles::from_hex(vec![1, 2, 3]);
        assert!(!n.skip_prefix(&prefix));
    }

    #[test]
    fn skip_prefix_true_same_length() {
        let mut n = Nibbles::from_hex(vec![1, 2, 3]);
        let prefix = Nibbles::from_hex(vec![1, 2, 3]);
        assert!(n.skip_prefix(&prefix));
        assert!(n.is_empty());
    }

    #[test]
    fn next_choice() {
        let mut n = Nibbles::from_hex(vec![5, 10, 16]);
        assert_eq!(n.next_choice(), Some(5));
        assert_eq!(n.next_choice(), Some(10));
        assert_eq!(n.next_choice(), None); // 16 is not a valid choice
    }

    #[test]
    fn offset_basic() {
        let n = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let o = n.offset(2);
        assert_eq!(o.to_hex(), vec![3, 4, 5]);
    }

    #[test]
    fn slice_basic() {
        let n = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let s = n.slice(1, 4);
        assert_eq!(s.to_hex(), vec![2, 3, 4]);
    }

    #[test]
    fn extend_basic() {
        let mut n = Nibbles::from_hex(vec![1, 2]);
        let other = Nibbles::from_hex(vec![3, 4]);
        n.extend(&other);
        assert_eq!(n.to_hex(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn extend_with_leaf_flag() {
        let mut n = Nibbles::from_hex(vec![1, 2]);
        let other = Nibbles::from_hex(vec![3, 4, 16]); // has leaf flag
        n.extend(&other);
        assert_eq!(n.to_hex(), vec![1, 2, 3, 4, 16]);
        assert!(n.is_leaf());
    }

    #[test]
    fn prepend_basic() {
        let mut n = Nibbles::from_hex(vec![3, 4]);
        let other = Nibbles::from_hex(vec![1, 2]);
        n.prepend(&other);
        assert_eq!(n.to_hex(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn prepend_nibble_basic() {
        let mut n = Nibbles::from_hex(vec![2, 3, 4]);
        n.prepend_nibble(1);
        assert_eq!(n.to_hex(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn prepend_nibble_to_empty() {
        let mut n = Nibbles::from_hex(vec![]);
        n.prepend_nibble(5);
        assert_eq!(n.to_hex(), vec![5]);
    }

    #[test]
    fn append_basic() {
        let mut n = Nibbles::from_hex(vec![1, 2]);
        n.append(3);
        assert_eq!(n.to_hex(), vec![1, 2, 3]);
    }

    #[test]
    fn concat_basic() {
        let a = Nibbles::from_hex(vec![1, 2]);
        let b = Nibbles::from_hex(vec![3, 4]);
        let c = a.concat(&b);
        assert_eq!(c.to_hex(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn equality() {
        let a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![1, 2, 3]);
        let c = Nibbles::from_hex(vec![1, 2, 4]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn ordering() {
        let a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![1, 2, 4]);
        let c = Nibbles::from_hex(vec![1, 2]);
        assert!(a < b);
        assert!(c < a);
    }

    #[test]
    fn count_prefix_all() {
        let a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![1, 2, 3]);
        assert_eq!(a.count_prefix(&b), 3);
    }

    #[test]
    fn count_prefix_partial() {
        let a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![1, 2, 4]);
        assert_eq!(a.count_prefix(&b), 2);
    }

    #[test]
    fn count_prefix_none() {
        let a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![4, 5, 6]);
        assert_eq!(a.count_prefix(&b), 0);
    }

    #[test]
    fn compare_prefix_equal() {
        let a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![1, 2, 3]);
        assert_eq!(a.compare_prefix(&b), cmp::Ordering::Equal);
    }

    #[test]
    fn compare_prefix_less() {
        let a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![1, 2, 4]);
        assert_eq!(a.compare_prefix(&b), cmp::Ordering::Less);
    }

    #[test]
    fn compare_prefix_greater() {
        let a = Nibbles::from_hex(vec![1, 2, 4]);
        let b = Nibbles::from_hex(vec![1, 2, 3]);
        assert_eq!(a.compare_prefix(&b), cmp::Ordering::Greater);
    }

    #[test]
    fn compare_prefix_equal_a_longer() {
        let a = Nibbles::from_hex(vec![1, 2, 3, 4]);
        let b = Nibbles::from_hex(vec![1, 2, 3]);
        assert_eq!(a.compare_prefix(&b), cmp::Ordering::Equal);
    }

    #[test]
    fn compare_prefix_equal_b_longer() {
        let a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![1, 2, 3, 4]);
        assert_eq!(a.compare_prefix(&b), cmp::Ordering::Equal);
    }
}
