use smallvec::SmallVec;
use std::{cmp, mem};

use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

/// Struct representing a list of nibbles (half-bytes)
///
/// Uses `data_offset` for O(1) front removal instead of O(n) Vec::remove(0).
/// Uses `SmallVec` for `already_consumed` to avoid heap allocation for short paths.
#[derive(
    Debug,
    Clone,
    Default,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Deserialize,
    rkyv::Serialize,
    rkyv::Archive,
)]
pub struct Nibbles {
    data: Vec<u8>,
    /// Offset into `data` — elements before this index have been consumed.
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    data_offset: usize,
    /// Parts of the path that have already been consumed (used for tracking
    /// current position when visiting nodes). See `current()`.
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    already_consumed: SmallVec<[u8; 64]>,
}

// NOTE: custom impls to ignore the `already_consumed` and `data_offset` fields

impl PartialEq for Nibbles {
    fn eq(&self, other: &Nibbles) -> bool {
        self.as_ref() == other.as_ref()
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
        self.as_ref().cmp(other.as_ref())
    }
}

impl std::hash::Hash for Nibbles {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

impl Nibbles {
    /// Create `Nibbles` from  hex-encoded nibbles
    pub fn from_hex(hex: Vec<u8>) -> Self {
        Self {
            data: hex,
            data_offset: 0,
            already_consumed: SmallVec::new(),
        }
    }

    /// Splits incoming bytes into nibbles and appends the leaf flag (a 16 nibble at the end)
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_raw(bytes, true)
    }

    /// Splits incoming bytes into nibbles and appends the leaf flag (a 16 nibble at the end) if is_leaf is true
    pub fn from_raw(bytes: &[u8], is_leaf: bool) -> Self {
        let cap = bytes.len() * 2 + if is_leaf { 1 } else { 0 };
        let mut data = Vec::with_capacity(cap);
        for byte in bytes {
            data.push(byte >> 4 & 0x0F);
            data.push(byte & 0x0F);
        }
        if is_leaf {
            data.push(16);
        }

        Self {
            data,
            data_offset: 0,
            already_consumed: SmallVec::new(),
        }
    }

    pub fn into_vec(self) -> Vec<u8> {
        if self.data_offset == 0 {
            self.data
        } else {
            self.data[self.data_offset..].to_vec()
        }
    }

    /// Returns the amount of nibbles
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len() - self.data_offset
    }

    /// Returns true if there are no nibbles
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data_offset >= self.data.len()
    }

    /// If `prefix` is a prefix of self, move the offset after
    /// the prefix and return true, otherwise return false.
    pub fn skip_prefix(&mut self, prefix: &Nibbles) -> bool {
        let remaining = &self.data[self.data_offset..];
        if remaining.len() >= prefix.len() && &remaining[..prefix.len()] == prefix.as_ref() {
            self.already_consumed
                .extend_from_slice(prefix.as_ref());
            self.data_offset += prefix.len();
            true
        } else {
            false
        }
    }

    /// Compares self to another, comparing prefixes only in case of unequal lengths.
    pub fn compare_prefix(&self, prefix: &Nibbles) -> cmp::Ordering {
        let data = self.as_ref();
        let prefix_data = prefix.as_ref();
        if data.len() > prefix_data.len() {
            data[..prefix_data.len()].cmp(prefix_data)
        } else {
            data.cmp(&prefix_data[..data.len()])
        }
    }

    /// Compares self to another and returns the shared nibble count (amount of nibbles that are equal, from the start)
    pub fn count_prefix(&self, other: &Nibbles) -> usize {
        self.as_ref()
            .iter()
            .zip(other.as_ref().iter())
            .take_while(|(a, b)| a == b)
            .count()
    }

    /// Removes and returns the first nibble
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<u8> {
        (self.data_offset < self.data.len()).then(|| {
            let nibble = self.data[self.data_offset];
            self.already_consumed.push(nibble);
            self.data_offset += 1;
            nibble
        })
    }

    /// Removes and returns the first nibble if it is a suitable choice index (aka < 16)
    pub fn next_choice(&mut self) -> Option<usize> {
        self.next().filter(|choice| *choice < 16).map(usize::from)
    }

    /// Returns the nibbles after the given offset
    pub fn offset(&self, offset: usize) -> Nibbles {
        let data_start = self.data_offset;
        let mut already_consumed =
            SmallVec::with_capacity(self.already_consumed.len() + offset);
        already_consumed.extend_from_slice(&self.already_consumed);
        already_consumed.extend_from_slice(&self.data[data_start..data_start + offset]);
        Nibbles {
            data: self.data[data_start + offset..].to_vec(),
            data_offset: 0,
            already_consumed,
        }
    }

    /// Returns the nibbles beween the start and end indexes
    pub fn slice(&self, start: usize, end: usize) -> Nibbles {
        let data_start = self.data_offset;
        Nibbles::from_hex(self.data[data_start + start..data_start + end].to_vec())
    }

    /// Extends the nibbles with another list of nibbles
    pub fn extend(&mut self, other: &Nibbles) {
        // If we have an offset, compact first to avoid fragmented data
        if self.data_offset > 0 {
            self.data = self.data[self.data_offset..].to_vec();
            self.data_offset = 0;
        }
        self.data.extend_from_slice(other.as_ref());
    }

    /// Return the nibble at the given index, will panic if the index is out of range
    pub fn at(&self, i: usize) -> usize {
        self.data[self.data_offset + i] as usize
    }

    /// Inserts a nibble at the start
    pub fn prepend(&mut self, nibble: u8) {
        if self.data_offset > 0 {
            // Reuse the slot before the current offset
            self.data_offset -= 1;
            self.data[self.data_offset] = nibble;
        } else {
            self.data.insert(0, nibble);
        }
    }

    /// Inserts a nibble at the end
    pub fn append(&mut self, nibble: u8) {
        self.data.push(nibble);
    }

    /// Taken from https://github.com/citahub/cita_trie/blob/master/src/nibbles.rs#L56
    /// Encodes the nibbles in compact form
    pub fn encode_compact(&self) -> Vec<u8> {
        let data = self.as_ref();
        let is_leaf = self.is_leaf();
        let mut hex = if is_leaf {
            &data[0..data.len() - 1]
        } else {
            &data[0..]
        };
        // node type    path length    |    prefix    hexchar
        // --------------------------------------------------
        // extension    even           |    0000      0x0
        // extension    odd            |    0001      0x1
        // leaf         even           |    0010      0x2
        // leaf         odd            |    0011      0x3
        let mut compact = Vec::with_capacity(hex.len() / 2 + 1);
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
            self.data[self.data.len() - 1] == 16
        }
    }

    /// Combines the nibbles into bytes, trimming the leaf flag if necessary
    pub fn to_bytes(&self) -> Vec<u8> {
        let data = self.as_ref();
        // Trim leaf flag
        let data = if !data.is_empty() && data[data.len() - 1] == 16 {
            &data[..data.len() - 1]
        } else {
            data
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
        let self_data = self.as_ref();
        let other_data = other.as_ref();
        let mut data = Vec::with_capacity(self_data.len() + other_data.len());
        data.extend_from_slice(self_data);
        data.extend_from_slice(other_data);
        Nibbles {
            data,
            data_offset: 0,
            already_consumed: self.already_consumed.clone(),
        }
    }

    /// Returns a copy of self with the nibble added at the end
    pub fn append_new(&self, nibble: u8) -> Nibbles {
        let self_data = self.as_ref();
        let mut data = Vec::with_capacity(self_data.len() + 1);
        data.extend_from_slice(self_data);
        data.push(nibble);
        Nibbles {
            data,
            data_offset: 0,
            already_consumed: self.already_consumed.clone(),
        }
    }

    /// Return already consumed parts of path
    pub fn current(&self) -> Nibbles {
        Nibbles {
            data: self.already_consumed.to_vec(),
            data_offset: 0,
            already_consumed: SmallVec::new(),
        }
    }

    /// Empties `self.data` and returns the content
    pub fn take(&mut self) -> Self {
        let data = if self.data_offset > 0 {
            let d = self.data[self.data_offset..].to_vec();
            self.data.clear();
            self.data_offset = 0;
            d
        } else {
            mem::take(&mut self.data)
        };
        Nibbles {
            data,
            data_offset: 0,
            already_consumed: mem::take(&mut self.already_consumed),
        }
    }
}

impl AsRef<[u8]> for Nibbles {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.data[self.data_offset..]
    }
}

impl RLPEncode for Nibbles {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        if self.data_offset == 0 {
            Encoder::new(buf).encode_field(&self.data).finish();
        } else {
            let active: Vec<u8> = self.data[self.data_offset..].to_vec();
            Encoder::new(buf).encode_field(&active).finish();
        }
    }
}

impl RLPDecode for Nibbles {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (data, decoder) = decoder.decode_field("data")?;
        Ok((
            Self {
                data,
                data_offset: 0,
                already_consumed: SmallVec::new(),
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
    let end = if base[0] < 2 {
        base.len() - 1
    } else {
        base.len()
    };
    // apply odd flag
    let chop = 2 - (base[0] & 1) as usize;
    base.drain(..chop);
    base.truncate(end - chop);
    base
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
