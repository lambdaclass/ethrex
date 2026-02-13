use std::{cmp, fmt};

use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

/// Maximum number of nibbles that can be stored.
/// 131 max (storage prefix key: 1 byte prefix + 32 byte storage key + 32 byte key = 65 bytes
/// = 130 nibbles + 1 prefix nibble) + 1 for the leaf flag = 132
const MAX_NIBBLES: usize = 132;

/// Struct representing a list of nibbles (half-bytes).
///
/// Uses a fixed-size stack buffer instead of heap-allocated Vecs.
/// - `buf[0..offset]` = consumed nibbles (path already traversed)
/// - `buf[offset..len]` = active nibbles (remaining path)
#[derive(Clone, Copy, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Nibbles {
    buf: [u8; MAX_NIBBLES],
    len: u8,
    offset: u8,
}

impl fmt::Debug for Nibbles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Nibbles")
            .field("data", &self.active())
            .field("consumed", &self.consumed())
            .finish()
    }
}

impl Default for Nibbles {
    fn default() -> Self {
        Self {
            buf: [0; MAX_NIBBLES],
            len: 0,
            offset: 0,
        }
    }
}

// NOTE: custom impls to ignore the consumed portion (same as old behavior ignoring `already_consumed`)

impl PartialEq for Nibbles {
    fn eq(&self, other: &Nibbles) -> bool {
        self.active() == other.active()
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
        self.active().cmp(other.active())
    }
}

impl std::hash::Hash for Nibbles {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.active().hash(state);
    }
}

impl Nibbles {
    /// Returns the active (unconsumed) nibbles
    fn active(&self) -> &[u8] {
        &self.buf[self.offset as usize..self.len as usize]
    }

    /// Returns the consumed nibbles
    fn consumed(&self) -> &[u8] {
        &self.buf[..self.offset as usize]
    }

    /// Create `Nibbles` from hex-encoded nibbles
    pub fn from_hex(hex: Vec<u8>) -> Self {
        debug_assert!(
            hex.len() <= MAX_NIBBLES,
            "nibbles overflow: {} > {MAX_NIBBLES}",
            hex.len(),
        );
        let mut nibbles = Self::default();
        let n = hex.len().min(MAX_NIBBLES);
        nibbles.buf[..n].copy_from_slice(&hex[..n]);
        nibbles.len = n as u8;
        nibbles
    }

    /// Splits incoming bytes into nibbles and appends the leaf flag (a 16 nibble at the end)
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_raw(bytes, true)
    }

    /// Splits incoming bytes into nibbles and appends the leaf flag (a 16 nibble at the end) if is_leaf is true
    pub fn from_raw(bytes: &[u8], is_leaf: bool) -> Self {
        debug_assert!(
            bytes.len() * 2 + is_leaf as usize <= MAX_NIBBLES,
            "nibbles overflow in from_raw: {} > {MAX_NIBBLES}",
            bytes.len() * 2 + is_leaf as usize,
        );
        let mut nibbles = Self::default();
        let mut pos = 0;
        for &byte in bytes {
            if pos + 2 > MAX_NIBBLES {
                break;
            }
            nibbles.buf[pos] = byte >> 4;
            nibbles.buf[pos + 1] = byte & 0x0F;
            pos += 2;
        }
        if is_leaf && pos < MAX_NIBBLES {
            nibbles.buf[pos] = 16;
            pos += 1;
        }
        nibbles.len = pos as u8;
        nibbles
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.active().to_vec()
    }

    /// Returns the amount of nibbles
    pub fn len(&self) -> usize {
        self.len as usize - self.offset as usize
    }

    /// Returns true if there are no nibbles
    pub fn is_empty(&self) -> bool {
        self.offset == self.len
    }

    /// If `prefix` is a prefix of self, move the offset after
    /// the prefix and return true, otherwise return false.
    pub fn skip_prefix(&mut self, prefix: &Nibbles) -> bool {
        let plen = prefix.len();
        if self.len() >= plen && self.active()[..plen] == *prefix.active() {
            self.offset += plen as u8;
            true
        } else {
            false
        }
    }

    /// Compares self to another, comparing prefixes only in case of unequal lengths.
    pub fn compare_prefix(&self, prefix: &Nibbles) -> cmp::Ordering {
        let active = self.active();
        let pactive = prefix.active();
        if active.len() > pactive.len() {
            active[..pactive.len()].cmp(pactive)
        } else {
            active.cmp(&pactive[..active.len()])
        }
    }

    /// Compares self to another and returns the shared nibble count (amount of nibbles that are equal, from the start)
    pub fn count_prefix(&self, other: &Nibbles) -> usize {
        self.active()
            .iter()
            .zip(other.active().iter())
            .take_while(|(a, b)| a == b)
            .count()
    }

    /// Removes and returns the first nibble
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<u8> {
        if self.is_empty() {
            None
        } else {
            let nibble = self.buf[self.offset as usize];
            self.offset += 1;
            Some(nibble)
        }
    }

    /// Removes and returns the first nibble if it is a suitable choice index (aka < 16)
    pub fn next_choice(&mut self) -> Option<usize> {
        self.next().filter(|choice| *choice < 16).map(usize::from)
    }

    /// Returns the nibbles after the given offset
    pub fn offset(&self, offset: usize) -> Nibbles {
        Nibbles {
            buf: self.buf,
            len: self.len,
            offset: self.offset + offset as u8,
        }
    }

    /// Returns the nibbles between the start and end indexes
    pub fn slice(&self, start: usize, end: usize) -> Nibbles {
        let active = self.active();
        let slice_data = &active[start..end];
        let mut result = Nibbles::default();
        let n = slice_data.len();
        result.buf[..n].copy_from_slice(slice_data);
        result.len = n as u8;
        result
    }

    /// Extends the nibbles with another list of nibbles
    pub fn extend(&mut self, other: &Nibbles) {
        let other_active = other.active();
        let end = self.len as usize;
        let o_len = other_active.len();
        self.buf[end..end + o_len].copy_from_slice(other_active);
        self.len += o_len as u8;
    }

    /// Return the nibble at the given index, will panic if the index is out of range
    pub fn at(&self, i: usize) -> usize {
        self.active()[i] as usize
    }

    /// Inserts a nibble at the start
    pub fn prepend(&mut self, nibble: u8) {
        let start = self.offset as usize;
        let end = self.len as usize;
        debug_assert!(end < MAX_NIBBLES, "nibbles overflow in prepend");
        self.buf.copy_within(start..end, start + 1);
        self.buf[start] = nibble;
        self.len += 1;
    }

    /// Inserts a nibble at the end
    pub fn append(&mut self, nibble: u8) {
        debug_assert!((self.len as usize) < MAX_NIBBLES, "nibbles overflow in append");
        self.buf[self.len as usize] = nibble;
        self.len += 1;
    }

    /// Taken from https://github.com/citahub/cita_trie/blob/master/src/nibbles.rs#L56
    /// Encodes the nibbles in compact form
    pub fn encode_compact(&self) -> Vec<u8> {
        let active = self.active();
        let mut compact = vec![];
        let is_leaf = self.is_leaf();
        let mut hex = if is_leaf {
            &active[..active.len() - 1]
        } else {
            active
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

    /// Decodes the nibbles from compact form
    pub fn decode_compact(compact: &[u8]) -> Self {
        Self::from_hex(compact_to_hex(compact))
    }

    /// Returns true if the nibbles contain the leaf flag (16) at the end
    pub fn is_leaf(&self) -> bool {
        let active = self.active();
        !active.is_empty() && active[active.len() - 1] == 16
    }

    /// Combines the nibbles into bytes, trimming the leaf flag if necessary
    pub fn to_bytes(&self) -> Vec<u8> {
        let active = self.active();
        // Trim leaf flag
        let data = if !active.is_empty() && self.is_leaf() {
            &active[..active.len() - 1]
        } else {
            active
        };
        // Combine nibbles into bytes
        data.chunks(2)
            .map(|chunk| match chunk.len() {
                1 => chunk[0] << 4,
                _ => chunk[0] << 4 | chunk[1],
            })
            .collect::<Vec<_>>()
    }

    /// Concatenates self and another Nibbles returning a new Nibbles
    pub fn concat(&self, other: &Nibbles) -> Nibbles {
        let mut result = Nibbles::default();
        let consumed = self.consumed();
        let self_active = self.active();
        let other_active = other.active();
        let c_len = consumed.len();
        let s_len = self_active.len();
        let o_len = other_active.len();
        let total = c_len + s_len + o_len;
        result.buf[..c_len].copy_from_slice(consumed);
        result.buf[c_len..c_len + s_len].copy_from_slice(self_active);
        result.buf[c_len + s_len..total].copy_from_slice(other_active);
        result.offset = c_len as u8;
        result.len = total as u8;
        result
    }

    /// Returns a copy of self with the nibble added at the end
    pub fn append_new(&self, nibble: u8) -> Nibbles {
        let mut result = *self;
        result.buf[result.len as usize] = nibble;
        result.len += 1;
        result
    }

    /// Return already consumed parts of path
    pub fn current(&self) -> Nibbles {
        let consumed = self.consumed();
        let mut result = Nibbles::default();
        let c_len = consumed.len();
        result.buf[..c_len].copy_from_slice(consumed);
        result.len = c_len as u8;
        result
    }

    /// Empties `self` and returns the content
    pub fn take(&mut self) -> Self {
        let result = *self;
        *self = Self::default();
        result
    }
}

impl AsRef<[u8]> for Nibbles {
    fn as_ref(&self) -> &[u8] {
        self.active()
    }
}

impl RLPEncode for Nibbles {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.active().to_vec())
            .finish();
    }
}

impl RLPDecode for Nibbles {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (data, decoder) = decoder.decode_field("data")?;
        Ok((Self::from_hex(data), decoder.finish()?))
    }
}

// Custom serde impls to maintain wire-format compatibility with the old
// `{ "data": [...], "already_consumed": [...] }` layout.
impl serde::Serialize for Nibbles {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Nibbles", 2)?;
        let data: Vec<u8> = self.active().to_vec();
        let consumed: Vec<u8> = self.consumed().to_vec();
        state.serialize_field("data", &data)?;
        state.serialize_field("already_consumed", &consumed)?;
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for Nibbles {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct NibblesHelper {
            data: Vec<u8>,
            already_consumed: Vec<u8>,
        }
        let helper = NibblesHelper::deserialize(deserializer)?;
        let consumed_len = helper.already_consumed.len();
        let active_len = helper.data.len();
        let total = consumed_len + active_len;
        let mut nibbles = Self::default();
        nibbles.buf[..consumed_len].copy_from_slice(&helper.already_consumed);
        nibbles.buf[consumed_len..total].copy_from_slice(&helper.data);
        nibbles.offset = consumed_len as u8;
        nibbles.len = total as u8;
        Ok(nibbles)
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
