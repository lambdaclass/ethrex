use std::cmp;

use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

// Max 131 nibbles observed (from apply_prefix in layering.rs), rounded up.
const MAX_NIBBLES: usize = 132;

// TODO: move path-tracking logic somewhere else
/// Stack-allocated nibble sequence for trie traversal.
///
/// Layout: `buf[0..start]` is the already-consumed prefix (for path tracking),
/// and `buf[start..start+len]` is the remaining data. This makes `next()` and
/// `skip_prefix()` O(1) by advancing `start` instead of shifting a Vec.
#[derive(Debug, Clone, rkyv::Deserialize, rkyv::Serialize, rkyv::Archive)]
pub struct Nibbles {
    buf: [u8; MAX_NIBBLES],
    /// Index where remaining data begins.
    start: u8,
    /// Number of remaining data nibbles.
    len: u8,
}

impl Default for Nibbles {
    fn default() -> Self {
        Self {
            buf: [0u8; MAX_NIBBLES],
            start: 0,
            len: 0,
        }
    }
}

// NOTE: custom impls to ignore the `already_consumed` portion

impl PartialEq for Nibbles {
    fn eq(&self, other: &Nibbles) -> bool {
        self.data() == other.data()
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
        self.data().cmp(other.data())
    }
}

impl std::hash::Hash for Nibbles {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.data().hash(state);
    }
}

impl Nibbles {
    /// Returns the remaining data as a slice.
    #[inline]
    fn data(&self) -> &[u8] {
        &self.buf[self.start as usize..(self.start + self.len) as usize]
    }

    /// Create `Nibbles` from hex-encoded nibbles
    pub fn from_hex(hex: Vec<u8>) -> Self {
        Self::from_slice(&hex)
    }

    /// Create `Nibbles` from a nibble slice
    pub fn from_slice(data: &[u8]) -> Self {
        debug_assert!(data.len() <= MAX_NIBBLES);
        let mut buf = [0u8; MAX_NIBBLES];
        buf[..data.len()].copy_from_slice(data);
        Self {
            buf,
            start: 0,
            len: data.len() as u8,
        }
    }

    /// Splits incoming bytes into nibbles and appends the leaf flag (a 16 nibble at the end)
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_raw(bytes, true)
    }

    /// Splits incoming bytes into nibbles and appends the leaf flag (a 16 nibble at the end) if is_leaf is true
    pub fn from_raw(bytes: &[u8], is_leaf: bool) -> Self {
        let nibble_count = bytes.len() * 2 + if is_leaf { 1 } else { 0 };
        debug_assert!(nibble_count <= MAX_NIBBLES);
        let mut buf = [0u8; MAX_NIBBLES];
        for (i, byte) in bytes.iter().enumerate() {
            buf[i * 2] = byte >> 4 & 0x0F;
            buf[i * 2 + 1] = byte & 0x0F;
        }
        if is_leaf {
            buf[bytes.len() * 2] = 16;
        }
        Self {
            buf,
            start: 0,
            len: nibble_count as u8,
        }
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.data().to_vec()
    }

    /// Returns the amount of nibbles
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Returns true if there are no nibbles
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// If `prefix` is a prefix of self, move the offset after
    /// the prefix and return true, otherwise return false.
    pub fn skip_prefix(&mut self, prefix: &Nibbles) -> bool {
        let prefix_len = prefix.len();
        if self.len() >= prefix_len && &self.data()[..prefix_len] == prefix.data() {
            self.start += prefix_len as u8;
            self.len -= prefix_len as u8;
            true
        } else {
            false
        }
    }

    /// Compares self to another, comparing prefixes only in case of unequal lengths.
    pub fn compare_prefix(&self, prefix: &Nibbles) -> cmp::Ordering {
        let data = self.data();
        let prefix_data = prefix.data();
        if data.len() > prefix_data.len() {
            data[..prefix_data.len()].cmp(prefix_data)
        } else {
            data[..].cmp(&prefix_data[..data.len()])
        }
    }

    /// Compares self to another and returns the shared nibble count (amount of nibbles that are equal, from the start)
    pub fn count_prefix(&self, other: &Nibbles) -> usize {
        self.data()
            .iter()
            .zip(other.data().iter())
            .take_while(|(a, b)| a == b)
            .count()
    }

    /// Removes and returns the first nibble
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<u8> {
        if self.len == 0 {
            None
        } else {
            let nibble = self.buf[self.start as usize];
            self.start += 1;
            self.len -= 1;
            Some(nibble)
        }
    }

    /// Removes and returns the first nibble if it is a suitable choice index (aka < 16)
    pub fn next_choice(&mut self) -> Option<usize> {
        self.next().filter(|choice| *choice < 16).map(usize::from)
    }

    /// Returns the nibbles after the given offset
    pub fn offset(&self, offset: usize) -> Nibbles {
        debug_assert!(offset <= self.len as usize);
        Nibbles {
            buf: self.buf,
            start: self.start + offset as u8,
            len: self.len - offset as u8,
        }
    }

    /// Returns the nibbles between the start and end indexes
    pub fn slice(&self, start: usize, end: usize) -> Nibbles {
        let abs_start = self.start as usize + start;
        let slice_len = end - start;
        let mut buf = [0u8; MAX_NIBBLES];
        buf[..slice_len].copy_from_slice(&self.buf[abs_start..abs_start + slice_len]);
        Nibbles {
            buf,
            start: 0,
            len: slice_len as u8,
        }
    }

    /// Extends the nibbles with another list of nibbles
    pub fn extend(&mut self, other: &Nibbles) {
        let end = (self.start + self.len) as usize;
        let other_data = other.data();
        self.buf[end..end + other_data.len()].copy_from_slice(other_data);
        self.len += other.len;
    }

    /// Return the nibble at the given index, will panic if the index is out of range
    pub fn at(&self, i: usize) -> usize {
        self.buf[self.start as usize + i] as usize
    }

    /// Inserts a nibble at the start
    pub fn prepend(&mut self, nibble: u8) {
        let start = self.start as usize;
        let end = start + self.len as usize;
        self.buf.copy_within(start..end, start + 1);
        self.buf[start] = nibble;
        self.len += 1;
    }

    /// Inserts a nibble at the end
    pub fn append(&mut self, nibble: u8) {
        self.buf[(self.start + self.len) as usize] = nibble;
        self.len += 1;
    }

    /// Taken from https://github.com/citahub/cita_trie/blob/master/src/nibbles.rs#L56
    /// Encodes the nibbles in compact form
    pub fn encode_compact(&self) -> Vec<u8> {
        let mut compact = vec![];
        let is_leaf = self.is_leaf();
        let data = self.data();
        let mut hex = if is_leaf {
            &data[..data.len() - 1]
        } else {
            data
        };
        // node type    path length    |    prefix    hexchar
        // --------------------------------------------------
        // extension    even           |    0000      0x0
        // extension    odd            |    0001      0x1
        // leaf         even           |    0010      0x2
        // leaf         odd            |    0011      0x3
        let v = if hex.len() % 2 == 1 {
            let v = 0x10 + hex[0];
            hex = &hex[1..];
            v
        } else {
            0x00
        };

        compact.push(v + if is_leaf { 0x20 } else { 0x00 });
        for i in 0..(hex.len() / 2) {
            compact.push((hex[i * 2] * 16) + (hex[i * 2 + 1]));
        }

        compact
    }

    /// Encodes the nibbles in compact form
    pub fn decode_compact(compact: &[u8]) -> Self {
        Self::from_hex(compact_to_hex(compact))
    }

    /// Returns true if the nibbles contain the leaf flag (16) at the end
    pub fn is_leaf(&self) -> bool {
        if self.is_empty() {
            false
        } else {
            self.buf[(self.start + self.len - 1) as usize] == 16
        }
    }

    /// Combines the nibbles into bytes, trimming the leaf flag if necessary
    pub fn to_bytes(&self) -> Vec<u8> {
        let data = self.data();
        // Trim leaf flag
        let trimmed = if !self.is_empty() && self.is_leaf() {
            &data[..data.len() - 1]
        } else {
            data
        };
        // Combine nibbles into bytes
        trimmed
            .chunks(2)
            .map(|chunk| match chunk.len() {
                1 => chunk[0] << 4,
                _ => chunk[0] << 4 | chunk[1],
            })
            .collect::<Vec<_>>()
    }

    /// Concatenates self and another Nibbles returning a new Nibbles
    pub fn concat(&self, other: &Nibbles) -> Nibbles {
        let mut buf = self.buf;
        let end = (self.start + self.len) as usize;
        let other_data = other.data();
        buf[end..end + other_data.len()].copy_from_slice(other_data);
        Nibbles {
            buf,
            start: self.start,
            len: self.len + other.len,
        }
    }

    /// Returns a copy of self with the nibble added at the end
    pub fn append_new(&self, nibble: u8) -> Nibbles {
        let mut buf = self.buf;
        buf[(self.start + self.len) as usize] = nibble;
        Nibbles {
            buf,
            start: self.start,
            len: self.len + 1,
        }
    }

    /// Return already consumed parts of path
    pub fn current(&self) -> Nibbles {
        let consumed = self.start as usize;
        let mut buf = [0u8; MAX_NIBBLES];
        buf[..consumed].copy_from_slice(&self.buf[..consumed]);
        Nibbles {
            buf,
            start: 0,
            len: self.start,
        }
    }

    /// Empties `self.data` and returns the content
    pub fn take(&mut self) -> Self {
        let taken = self.clone();
        *self = Self::default();
        taken
    }
}

impl AsRef<[u8]> for Nibbles {
    fn as_ref(&self) -> &[u8] {
        self.data()
    }
}

// Custom serde impls to serialize only the meaningful data, not the full buffer.

impl serde::Serialize for Nibbles {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("Nibbles", 2)?;
        s.serialize_field("data", self.data())?;
        s.serialize_field("already_consumed", &self.buf[..self.start as usize])?;
        s.end()
    }
}

impl<'de> serde::Deserialize<'de> for Nibbles {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Helper {
            data: Vec<u8>,
            already_consumed: Vec<u8>,
        }
        let h = Helper::deserialize(deserializer)?;
        let consumed_len = h.already_consumed.len();
        let data_len = h.data.len();
        let mut buf = [0u8; MAX_NIBBLES];
        buf[..consumed_len].copy_from_slice(&h.already_consumed);
        buf[consumed_len..consumed_len + data_len].copy_from_slice(&h.data);
        Ok(Nibbles {
            buf,
            start: consumed_len as u8,
            len: data_len as u8,
        })
    }
}

impl RLPEncode for Nibbles {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.data().to_vec())
            .finish();
    }
}

impl RLPDecode for Nibbles {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (data, decoder): (Vec<u8>, _) = decoder.decode_field("data")?;
        Ok((Self::from_hex(data), decoder.finish()?))
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
