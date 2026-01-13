#![allow(dead_code, unused_imports)]
use std::{
    cmp::{self, Ordering},
    hash::Hash,
    mem,
};

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

#[cfg(test)]
mod test {
    use super::*;
    use std::cmp::Ordering;
    use std::hash::{DefaultHasher, Hash, Hasher};

    #[test]
    fn skip_prefix_true() {
        let mut a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3]);
        assert!(a.skip_prefix(&b));
        assert_eq!(a.as_ref(), &[4, 5])
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
        assert_eq!(a.as_ref(), &[1, 2, 3])
    }

    #[test]
    fn skip_prefix_false() {
        let mut a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 4]);
        assert!(!a.skip_prefix(&b));
        assert_eq!(a.as_ref(), &[1, 2, 3, 4, 5])
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

    #[test]
    fn hash_nibble() {
        let a = Nibbles::from_hex(vec![1, 2, 3]);
        let mut s = DefaultHasher::new();
        a.hash(&mut s);
        assert_eq!(s.finish(), 8086395815454877121);
    }

    #[test]
    fn compact_hash_nibble() {
        let a = CompactNibbles::from_hex(vec![1, 2, 3]);
        let mut s = DefaultHasher::new();
        a.hash(&mut s);
        assert_eq!(s.finish(), 8086395815454877121);
    }

    #[test]
    fn compact_to_bytes() {
        let a = Nibbles::from_hex(vec![0x0F, 0x0F, 0x0F]);
        let b = CompactNibbles::from_hex(vec![0x0F, 0x0F, 0x0F]);
        assert_eq!(a.to_bytes(), b.to_bytes());
    }

    #[test]
    fn compact_count_prefix_all() {
        let a = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert_eq!(a.count_prefix(&b), a.len());
    }

    #[test]
    fn compact_count_prefix_partial() {
        let a = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = CompactNibbles::from_hex(vec![1, 2, 3]);
        assert_eq!(a.count_prefix(&b), b.len());
    }

    #[test]
    fn compact_count_prefix_none() {
        let a = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = CompactNibbles::from_hex(vec![2, 3, 4, 5, 6]);
        assert_eq!(a.count_prefix(&b), 0);
    }

    #[test]
    fn compact_compare_prefix_equal() {
        let a = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Equal);
    }

    #[test]
    fn compact_compare_prefix_less() {
        let a = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = CompactNibbles::from_hex(vec![1, 2, 4, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Less);
    }

    #[test]
    fn compact_compare_prefix_greater() {
        let a = CompactNibbles::from_hex(vec![1, 2, 4, 4, 5]);
        let b = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Greater);
    }

    #[test]
    fn compact_compare_prefix_equal_b_longer() {
        let a = CompactNibbles::from_hex(vec![1, 2, 3]);
        let b = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Equal);
    }

    #[test]
    fn compact_compare_prefix_equal_a_longer() {
        let a = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = CompactNibbles::from_hex(vec![1, 2, 3]);
        assert_eq!(a.compare_prefix(&b), Ordering::Equal);
    }

    #[test]
    fn compact_skip_prefix_true() {
        let mut a = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = CompactNibbles::from_hex(vec![1, 2, 3]);
        assert!(a.skip_prefix(&b));
        assert_eq!(a.into_vec(), &[4, 5])
    }

    #[test]
    fn compact_skip_prefix_true_same_length() {
        let mut a = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert!(a.skip_prefix(&b));
        assert!(a.is_empty());
    }

    #[test]
    fn compact_skip_prefix_longer_prefix() {
        let mut a = CompactNibbles::from_hex(vec![1, 2, 3]);
        let b = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert!(!a.skip_prefix(&b));
        assert_eq!(a.into_vec(), &[1, 2, 3])
    }

    #[test]
    fn compact_skip_prefix_false() {
        let mut a = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = CompactNibbles::from_hex(vec![1, 2, 4]);
        assert!(!a.skip_prefix(&b));
        assert_eq!(a.into_vec(), &[1, 2, 3, 4, 5])
    }

    #[test]
    fn compact_extend_odd_even() {
        let mut a = CompactNibbles::from_hex(vec![1, 2, 3]);
        let b = CompactNibbles::from_hex(vec![1, 2]);
        a.extend(&b);
        assert_eq!(a.into_vec(), &[1, 2, 3, 1, 2])
    }

    #[test]
    fn compact_extend_odd_odd() {
        let mut a = CompactNibbles::from_hex(vec![1, 2, 3]);
        let b = CompactNibbles::from_hex(vec![4]);
        a.extend(&b);
        assert_eq!(a.into_vec(), &[1, 2, 3, 4,])
    }

    #[test]
    fn compact_extend_even_odd() {
        let mut a = CompactNibbles::from_hex(vec![1, 2, 3]);
        let b = CompactNibbles::from_hex(vec![4]);
        a.extend(&b);
        assert_eq!(a.into_vec(), &[1, 2, 3, 4,])
    }

    #[test]
    fn compact_from_raw_and_leaf_flags() {
        let bytes = vec![0xAB, 0xCD];
        let raw = CompactNibbles::from_raw(&bytes, false);
        assert_eq!(raw.len(), 4);
        assert!(!raw.is_leaf());
        assert_eq!(raw.to_bytes(), bytes);
        assert_eq!(raw.into_vec(), vec![0xA, 0xB, 0xC, 0xD]);

        let leaf = CompactNibbles::from_bytes(&bytes);
        assert_eq!(leaf.len(), 4);
        assert!(leaf.is_leaf());
        assert_eq!(leaf.into_vec(), vec![0xA, 0xB, 0xC, 0xD]);

        let empty = CompactNibbles::from_hex(vec![]);
        assert!(empty.is_empty());
        assert!(!empty.is_leaf());
    }

    #[test]
    fn compact_next_and_choice_update_state() {
        let mut n = CompactNibbles::from_hex(vec![1, 2, 3]);
        assert_eq!(n.len(), 3);
        assert_eq!(n.next(), Some(1));
        assert_eq!(n.len(), 2);
        assert_eq!(n.current().into_vec(), vec![1]);
        assert_eq!(n.next_choice(), Some(2));
        assert_eq!(n.len(), 1);
        assert_eq!(n.current().into_vec(), vec![1, 2]);
        assert_eq!(n.next(), Some(3));
        assert!(n.is_empty());
        assert_eq!(n.next(), None);
    }

    #[test]
    fn compact_slice_offset_at() {
        let n = CompactNibbles::from_hex(vec![0xA, 0xB, 0xC, 0xD, 0xE]);
        assert_eq!(n.at(0), 0xA);
        assert_eq!(n.at(1), 0xB);
        assert_eq!(n.at(4), 0xE);

        let slice = n.slice(1, 4);
        assert_eq!(slice.into_vec(), vec![0xB, 0xC, 0xD]);

        let offset = n.offset(2);
        assert_eq!(offset.into_vec(), vec![0xC, 0xD, 0xE]);
    }

    #[test]
    fn compact_prepend_append() {
        let mut n = CompactNibbles::from_hex(vec![1, 2, 3]);
        n.prepend(4);
        n.append(5);
        assert_eq!(n.into_vec(), vec![4, 1, 2, 3, 5]);
    }

    #[test]
    fn compact_concat_and_append_new() {
        let a = CompactNibbles::from_hex(vec![1, 2]);
        let b = CompactNibbles::from_hex(vec![3, 4]);
        let c = a.concat(&b);
        assert_eq!(c.into_vec(), vec![1, 2, 3, 4]);

        let appended = a.append_new(5);
        assert_eq!(appended.into_vec(), vec![1, 2, 5]);
        assert_eq!(a.into_vec(), vec![1, 2]);
    }

    #[test]
    fn compact_take_clears_self() {
        let mut n = CompactNibbles::from_raw(&[0xAB, 0xCD], true);
        let taken = n.take();
        assert!(n.is_empty());
        assert_eq!(n.len(), 0);
        assert!(!n.is_leaf());
        assert_eq!(taken.to_bytes(), vec![0xAB, 0xCD]);
        assert!(taken.is_leaf());
    }

    #[test]
    fn compact_encode_compact_matches_nibbles() {
        let nibbles = vec![1, 2, 3, 4, 5];
        let regular = Nibbles::from_hex(nibbles.clone());
        let compact = CompactNibbles::from_hex(nibbles);
        assert_eq!(compact.encode_compact(), regular.encode_compact());
    }

    #[test]
    fn compact_decode_compact_leaf_roundtrip() {
        let bytes = vec![0xAB, 0xCD];
        let compact = CompactNibbles::from_raw(&bytes, true);
        let encoded = compact.encode_compact();
        let decoded = CompactNibbles::decode_compact(&encoded);
        assert_eq!(decoded.to_bytes(), bytes);
        assert!(decoded.is_leaf());
    }

    #[test]
    fn compact_decode_compact_empty() {
        let decoded = CompactNibbles::decode_compact(&[]);
        assert!(decoded.is_empty());
        assert!(!decoded.is_leaf());
        assert_eq!(decoded.to_bytes(), Vec::<u8>::new());
    }

    #[test]
    fn compact_encode_decode_compact_odd_leaf() {
        let mut compact = CompactNibbles::from_raw(&[0x12], true);
        compact.append(0x3);
        let encoded = compact.encode_compact();
        let decoded = CompactNibbles::decode_compact(&encoded);
        assert!(decoded.is_leaf());
        assert_eq!(decoded.into_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn compact_skip_prefix_updates_current() {
        let mut n = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let prefix = CompactNibbles::from_hex(vec![1, 2, 3]);
        assert!(n.skip_prefix(&prefix));
        assert_eq!(n.current().into_vec(), vec![1, 2, 3]);
        assert_eq!(n.into_vec(), vec![4, 5]);
    }

    #[test]
    fn compact_offset_updates_current() {
        let n = CompactNibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let offset = n.offset(2);
        assert_eq!(offset.current().into_vec(), vec![1, 2]);
        assert_eq!(offset.into_vec(), vec![3, 4, 5]);
    }
}

fn compact(mut hex: Vec<u8>) -> Vec<u8> {
    let mut l = 0;
    let mut r = 0;
    while r < hex.len() {
        hex[l] = hex[r] << 4;
        if r < hex.len() - 1 {
            hex[l] |= hex[r + 1] & 0x0F;
            r += 1;
        }
        l += 1;
        r += 1;
    }
    hex.truncate(l);
    hex
}

fn expand(bytes: &[u8], len: usize) -> Vec<u8> {
    let mut it = bytes.iter().peekable();
    let odd_cleanup = len % 2 == 1;
    let mut res = Vec::with_capacity(len);

    while let Some(b) = it.next() {
        let is_last = it.peek().is_none();
        res.push(b >> 4);
        if !(odd_cleanup && is_last) {
            res.push(b & 0x0F);
        }
    }
    res
}

#[derive(Debug, Clone)]
struct CompactNibbles {
    len: usize,
    data: Vec<u8>,
    already_consumed: Vec<u8>,
    is_leaf: bool,
}

impl CompactNibbles {
    /// Create `Nibbles` from  hex-encoded nibbles
    pub fn from_hex(hex: Vec<u8>) -> Self {
        Self {
            len: hex.len(),
            data: compact(hex),
            already_consumed: vec![],
            is_leaf: false,
        }
    }

    /// Splits incoming bytes into nibbles and appends the leaf flag (a 16 nibble at the end)
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_raw(bytes, true)
    }

    /// Splits incoming bytes into nibbles and appends the leaf flag (a 16 nibble at the end) if is_leaf is true
    pub fn from_raw(bytes: &[u8], is_leaf: bool) -> Self {
        Self {
            data: bytes.to_vec(),
            len: bytes.len() * 2,
            already_consumed: vec![],
            is_leaf,
        }
    }

    pub fn into_vec(self) -> Vec<u8> {
        expand(&self.data, self.len)
    }

    /// Returns the amount of nibbles
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if there are no nibbles
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// If `prefix` is a prefix of self, move the offset after
    /// the prefix and return true, otherwise return false.
    pub fn skip_prefix(&mut self, prefix: &Self) -> bool {
        let prefix_len = self.count_prefix(prefix);
        if prefix_len == prefix.len() {
            let expanded = expand(&self.data, self.len)[prefix_len..].to_vec();
            self.len = expanded.len();
            self.data = compact(expanded);
            let prefix_nibbles = expand(&prefix.data, prefix.len);
            self.already_consumed.extend(prefix_nibbles);
            true
        } else {
            false
        }
    }

    pub fn compare_prefix(&self, prefix: &Self) -> Ordering {
        let odd_cleanup = self.len().min(prefix.len()) % 2 == 1;
        let mut it = self.data.iter().zip(prefix.data.iter()).peekable();

        while let Some((b1, b2)) = it.next() {
            let is_last = it.peek().is_none();
            let mut ord = b1.cmp(b2);

            if odd_cleanup && is_last {
                ord = (b1 & 0xF0).cmp(&(b2 & 0xF0))
            }
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    }

    /// Compares self to another and returns the shared nibble count (amount of nibbles that are equal, from the start)
    pub fn count_prefix(&self, other: &Self) -> usize {
        let odd_cleanup = self.len().min(other.len()) % 2 == 1;
        let mut it = self.data.iter().zip(other.data.iter()).peekable();
        let mut count = 0;

        while let Some((b1, b2)) = it.next() {
            let is_last = it.peek().is_none();
            let mut ord = b1.cmp(b2);

            if odd_cleanup && is_last {
                ord = (b1 & 0xF0).cmp(&(b2 & 0xF0))
            }
            if ord != Ordering::Equal {
                break;
            }
            count += if odd_cleanup && is_last { 1 } else { 2 };
        }
        count
    }

    fn shl(&mut self) -> Option<u8> {
        if self.is_empty() {
            None
        } else {
            let l = self.data[0] >> 4;
            for b in 0..self.data.len() {
                self.data[b] <<= 4;
                if b < self.data.len() - 1 {
                    self.data[b] |= self.data[b + 1] >> 4;
                    self.data[b + 1] &= 0x0F;
                }
            }
            Some(l)
        }
    }

    /// Removes and returns the first nibble
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<u8> {
        let l = self.shl()?;
        self.len = self.len.saturating_sub(1);
        let target_len = (self.len + 1) / 2;
        self.data.truncate(target_len);
        self.already_consumed.push(l);
        Some(l)
    }

    /// Removes and returns the first nibble if it is a suitable choice index (aka < 16)
    pub fn next_choice(&mut self) -> Option<usize> {
        self.next().filter(|choice| *choice < 16).map(usize::from)
    }

    /// Returns the nibbles after the given offset
    pub fn offset(&self, offset: usize) -> Self {
        let mut ret = self.slice(offset, self.len());
        let prefix = expand(&self.data, self.len)[0..offset].to_vec();
        ret.already_consumed =
            [self.already_consumed.as_slice(), prefix.as_slice()].concat();
        ret
    }

    /// Returns the nibbles between the start and end indexes
    pub fn slice(&self, start: usize, end: usize) -> Self {
        Self::from_hex(expand(&self.data, self.len)[start..end].to_vec())
    }

    /// Extends the nibbles with another list of nibbles
    pub fn extend(&mut self, other: &Self) {
        if other.is_empty() {
            return;
        }
        let odd_len = self.len % 2 == 1;
        self.data.reserve(other.data.len());
        if odd_len {
            let mut l = self.data.len() - 1;
            let mut r = 0;
            while r < other.data.len() {
                self.data[l] |= other.data[r] >> 4;
                self.data.push(other.data[r] << 4);
                l += 1;
                r += 1
            }
            if other.len % 2 == 1 {
                self.data.pop();
            }
        } else {
            self.data.extend(&other.data);
        }
        self.len += other.len();
    }

    /// Return the nibble at the given index, will panic if the index is out of range
    pub fn at(&self, i: usize) -> usize {
        if i.is_multiple_of(2) {
            (self.data[i / 2] >> 4) as usize
        } else {
            (self.data[i / 2] & 0x0F) as usize
        }
    }

    /// Inserts a nibble at the start
    pub fn prepend(&mut self, nibble: u8) {
        let odd_len = self.len % 2 == 1;
        self.data.insert(0, nibble << 4);
        self.len += 1;
        for l in 0..self.data.len() - 1 {
            self.data[l] |= self.data[l + 1] >> 4;
            self.data[l + 1] <<= 4;
        }
        if odd_len {
            self.data.pop();
        }
    }

    /// Inserts a nibble at the end
    pub fn append(&mut self, nibble: u8) {
        let odd_len = self.len % 2 == 1;
        if odd_len {
            let last = self.data.len() - 1;
            self.data[last] |= nibble & 0x0F;
        } else {
            self.data.push(nibble << 4);
        }
        self.len += 1;
    }

    /// Encodes the nibbles in compact form
    pub fn encode_compact(&self) -> Vec<u8> {
        let mut compact = vec![];
        let is_leaf = self.is_leaf();
        let mut hex = expand(&self.data, self.len);
        // node type    path length    |    prefix    hexchar
        // --------------------------------------------------
        // extension    even           |    0000      0x0
        // extension    odd            |    0001      0x1
        // leaf         even           |    0010      0x2
        // leaf         odd            |    0011      0x3
        let v = if hex.len() % 2 == 1 {
            let v = 0x10 + hex[0];
            hex = hex[1..].to_vec();
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
        let mut hex = compact_to_hex(compact);
        let is_leaf = matches!(hex.last(), Some(16));
        if is_leaf {
            hex.pop();
        }
        let mut nibbles = Self::from_hex(hex);
        nibbles.is_leaf = is_leaf && !nibbles.is_empty();
        nibbles
    }

    /// Returns true if the nibbles contain the leaf flag (16) at the end
    pub fn is_leaf(&self) -> bool {
        if self.is_empty() { false } else { self.is_leaf }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.data.clone()
    }

    /// Concatenates self and another Nibbles returning a new Nibbles
    pub fn concat(&self, other: &Self) -> Self {
        let mut n = self.clone();
        n.extend(other);
        n
    }

    /// Returns a copy of self with the nibble added at the and
    pub fn append_new(&self, nibble: u8) -> Self {
        let mut n = self.clone();
        n.append(nibble);
        n
    }

    /// Return already consumed parts of path
    pub fn current(&self) -> Self {
        Self::from_hex(self.already_consumed.clone())
    }

    /// Empties `self.data` and returns the content
    pub fn take(&mut self) -> Self {
        CompactNibbles {
            data: mem::take(&mut self.data),
            already_consumed: mem::take(&mut self.already_consumed),
            len: mem::take(&mut self.len),
            is_leaf: mem::take(&mut self.is_leaf),
        }
    }
}

impl std::hash::Hash for CompactNibbles {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let mut cur = 0;
        let mut words = vec![];
        while cur < self.len {
            let pos = cur % 2;
            let word = if pos == 1 {
                self.data[cur / 2] & 0x0F
            } else {
                self.data[cur / 2] >> 4
            };
            words.push(word);
            cur += 1;
        }
        words.hash(state);
    }
}
