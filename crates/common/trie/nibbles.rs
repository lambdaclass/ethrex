use std::{cmp, mem};

use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

// TODO: move path-tracking logic somewhere else
// PERF: try using a stack-allocated array
/// Struct representing a list of nibbles (half-bytes)
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
    /// Parts of the path that have already been consumed (used for tracking
    /// current position when visiting nodes). See `current()`.
    already_consumed: Vec<u8>,
}

// NOTE: custom impls to ignore the `already_consumed` field

impl PartialEq for Nibbles {
    fn eq(&self, other: &Nibbles) -> bool {
        self.data == other.data
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
        self.data.cmp(&other.data)
    }
}

impl std::hash::Hash for Nibbles {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.data.hash(state);
    }
}

impl Nibbles {
    /// Create `Nibbles` from  hex-encoded nibbles
    pub const fn from_hex(hex: Vec<u8>) -> Self {
        Self {
            data: hex,
            already_consumed: vec![],
        }
    }

    /// Splits incoming bytes into nibbles and appends the leaf flag (a 16 nibble at the end)
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_raw(bytes, true)
    }

    /// Splits incoming bytes into nibbles and appends the leaf flag (a 16 nibble at the end) if is_leaf is true
    pub fn from_raw(bytes: &[u8], is_leaf: bool) -> Self {
        let mut data: Vec<u8> = bytes
            .iter()
            .flat_map(|byte| [(byte >> 4 & 0x0F), byte & 0x0F])
            .collect();
        if is_leaf {
            data.push(16);
        }

        Self {
            data,
            already_consumed: vec![],
        }
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.data
    }

    /// Returns the amount of nibbles
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns true if there are no nibbles
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// If `prefix` is a prefix of self, move the offset after
    /// the prefix and return true, otherwise return false.
    pub fn skip_prefix(&mut self, prefix: &Nibbles) -> bool {
        if self.len() >= prefix.len() && &self.data[..prefix.len()] == prefix.as_ref() {
            self.data = self.data[prefix.len()..].to_vec();
            self.already_consumed.extend(&prefix.data);
            true
        } else {
            false
        }
    }

    /// Compares self to another, comparing prefixes only in case of unequal lengths.
    pub fn compare_prefix(&self, prefix: &Nibbles) -> cmp::Ordering {
        if self.len() > prefix.len() {
            self.data[..prefix.len()].cmp(&prefix.data)
        } else {
            self.data[..].cmp(&prefix.data[..self.len()])
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
        (!self.is_empty()).then(|| {
            self.already_consumed.push(self.data[0]);
            self.data.remove(0)
        })
    }

    /// Removes and returns the first nibble if it is a suitable choice index (aka < 16)
    pub fn next_choice(&mut self) -> Option<usize> {
        self.next().filter(|choice| *choice < 16).map(usize::from)
    }

    /// Returns the nibbles after the given offset
    pub fn offset(&self, offset: usize) -> Nibbles {
        let mut ret = self.slice(offset, self.len());
        ret.already_consumed = [&self.already_consumed, &self.data[0..offset]].concat();
        ret
    }

    /// Returns the nibbles beween the start and end indexes
    pub fn slice(&self, start: usize, end: usize) -> Nibbles {
        Nibbles::from_hex(self.data[start..end].to_vec())
    }

    /// Extends the nibbles with another list of nibbles
    pub fn extend(&mut self, other: &Nibbles) {
        self.data.extend_from_slice(other.as_ref());
    }

    /// Return the nibble at the given index, will panic if the index is out of range
    pub fn at(&self, i: usize) -> usize {
        self.data[i] as usize
    }

    /// Inserts a nibble at the start
    pub fn prepend(&mut self, nibble: u8) {
        self.data.insert(0, nibble);
    }

    /// Inserts a nibble at the end
    pub fn append(&mut self, nibble: u8) {
        self.data.push(nibble);
    }

    /// Taken from https://github.com/citahub/cita_trie/blob/master/src/nibbles.rs#L56
    /// Encodes the nibbles in compact form
    pub fn encode_compact(&self) -> Vec<u8> {
        let mut compact = vec![];
        let is_leaf = self.is_leaf();
        let mut hex = if is_leaf {
            &self.data[0..self.data.len() - 1]
        } else {
            &self.data[0..]
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
            self.data[self.data.len() - 1] == 16
        }
    }

    /// Combines the nibbles into bytes, trimming the leaf flag if necessary
    pub fn to_bytes(&self) -> Vec<u8> {
        // Trim leaf flag
        let data = if !self.is_empty() && self.is_leaf() {
            &self.data[..self.len() - 1]
        } else {
            &self.data[..]
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
        Nibbles {
            data: [&self.data[..], &other.data[..]].concat(),
            already_consumed: self.already_consumed.clone(),
        }
    }

    /// Returns a copy of self with the nibble added at the and
    pub fn append_new(&self, nibble: u8) -> Nibbles {
        Nibbles {
            data: [self.data.clone(), vec![nibble]].concat(),
            already_consumed: self.already_consumed.clone(),
        }
    }

    /// Return already consumed parts of path
    pub fn current(&self) -> Nibbles {
        Nibbles {
            data: self.already_consumed.clone(),
            already_consumed: vec![],
        }
    }

    /// Empties `self.data` and returns the content
    pub fn take(&mut self) -> Self {
        Nibbles {
            data: mem::take(&mut self.data),
            already_consumed: mem::take(&mut self.already_consumed),
        }
    }
}

/// Encode nibble data in compact (hex-prefix) form into a reusable buffer.
/// Works with any `&[u8]` slice of nibbles (both `Nibbles` and `PathVec`).
pub fn encode_compact_into(data: &[u8], is_leaf: bool, buf: &mut Vec<u8>) {
    buf.clear();
    let hex = if is_leaf && !data.is_empty() {
        &data[..data.len() - 1]
    } else {
        data
    };
    let v = if hex.len() % 2 == 1 {
        let v = 0x10 + hex[0];
        buf.push(v + if is_leaf { 0x20 } else { 0x00 });
        1
    } else {
        buf.push(if is_leaf { 0x20 } else { 0x00 });
        0
    };
    for i in (v..hex.len()).step_by(2) {
        buf.push(hex[i] * 16 + hex[i + 1]);
    }
}

impl AsRef<[u8]> for Nibbles {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl RLPEncode for Nibbles {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf).encode_field(&self.data).finish();
    }
}

impl RLPDecode for Nibbles {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (data, decoder) = decoder.decode_field("data")?;
        Ok((
            Self {
                data,
                already_consumed: vec![],
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
