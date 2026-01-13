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
struct Nibbles {
    len: usize,
    data: Vec<u8>,
    already_consumed: Vec<u8>,
    is_leaf: bool,
}

// NOTE: custom impls to ignore the `already_consumed` field

impl PartialEq for Nibbles {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Nibbles {}

impl PartialOrd for Nibbles {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Nibbles {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_leaf = self.is_leaf();
        let other_leaf = other.is_leaf();
        let self_len = self.len + usize::from(self_leaf);
        let other_len = other.len + usize::from(other_leaf);
        let min_len = self_len.min(other_len);

        for idx in 0..min_len {
            let lhs = if idx < self.len {
                self.at(idx) as u8
            } else {
                16
            };
            let rhs = if idx < other.len {
                other.at(idx) as u8
            } else {
                16
            };
            match lhs.cmp(&rhs) {
                Ordering::Equal => {}
                ord => return ord,
            }
        }

        self_len.cmp(&other_len)
    }
}

impl Nibbles {
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
        let target_len = self.len.div_ceil(2);
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
        ret.already_consumed = [self.already_consumed.as_slice(), prefix.as_slice()].concat();
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
        let mut hex = super::byte_nibbles::compact_to_hex(compact);
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
        Nibbles {
            data: mem::take(&mut self.data),
            already_consumed: mem::take(&mut self.already_consumed),
            len: mem::take(&mut self.len),
            is_leaf: mem::take(&mut self.is_leaf),
        }
    }
}

impl std::hash::Hash for Nibbles {
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

impl RLPEncode for Nibbles {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let mut hex = expand(&self.data, self.len);
        if self.is_leaf() {
            hex.push(16);
        }
        Encoder::new(buf).encode_field(&hex).finish();
    }
}

impl RLPDecode for Nibbles {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (mut data, decoder) = decoder.decode_field::<Vec<u8>>("data")?;
        let is_leaf = matches!(data.last(), Some(16));
        if is_leaf {
            data.pop();
        }
        let mut nibbles = Self::from_hex(data);
        nibbles.is_leaf = is_leaf && !nibbles.is_empty();
        Ok((nibbles, decoder.finish()?))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
    use std::cmp::Ordering;
    use std::hash::{DefaultHasher, Hash, Hasher};

    #[test]
    fn compact_hash_nibble() {
        let a = Nibbles::from_hex(vec![1, 2, 3]);
        let mut s = DefaultHasher::new();
        a.hash(&mut s);
        assert_eq!(s.finish(), 8086395815454877121);
    }

    #[test]
    fn compact_count_prefix_all() {
        let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert_eq!(a.count_prefix(&b), a.len());
    }

    #[test]
    fn compact_count_prefix_partial() {
        let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3]);
        assert_eq!(a.count_prefix(&b), b.len());
    }

    #[test]
    fn compact_count_prefix_none() {
        let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![2, 3, 4, 5, 6]);
        assert_eq!(a.count_prefix(&b), 0);
    }

    #[test]
    fn compact_compare_prefix_equal() {
        let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Equal);
    }

    #[test]
    fn compact_compare_prefix_less() {
        let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 4, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Less);
    }

    #[test]
    fn compact_compare_prefix_greater() {
        let a = Nibbles::from_hex(vec![1, 2, 4, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Greater);
    }

    #[test]
    fn compact_compare_prefix_equal_b_longer() {
        let a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Equal);
    }

    #[test]
    fn compact_compare_prefix_equal_a_longer() {
        let a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3]);
        assert_eq!(a.compare_prefix(&b), Ordering::Equal);
    }

    #[test]
    fn compact_skip_prefix_true() {
        let mut a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3]);
        assert!(a.skip_prefix(&b));
        assert_eq!(a.into_vec(), &[4, 5])
    }

    #[test]
    fn compact_skip_prefix_true_same_length() {
        let mut a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert!(a.skip_prefix(&b));
        assert!(a.is_empty());
    }

    #[test]
    fn compact_skip_prefix_longer_prefix() {
        let mut a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        assert!(!a.skip_prefix(&b));
        assert_eq!(a.into_vec(), &[1, 2, 3])
    }

    #[test]
    fn compact_skip_prefix_false() {
        let mut a = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(vec![1, 2, 4]);
        assert!(!a.skip_prefix(&b));
        assert_eq!(a.into_vec(), &[1, 2, 3, 4, 5])
    }

    #[test]
    fn compact_extend_odd_even() {
        let mut a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![1, 2]);
        a.extend(&b);
        assert_eq!(a.into_vec(), &[1, 2, 3, 1, 2])
    }

    #[test]
    fn compact_extend_odd_odd() {
        let mut a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![4]);
        a.extend(&b);
        assert_eq!(a.into_vec(), &[1, 2, 3, 4,])
    }

    #[test]
    fn compact_extend_even_odd() {
        let mut a = Nibbles::from_hex(vec![1, 2, 3]);
        let b = Nibbles::from_hex(vec![4]);
        a.extend(&b);
        assert_eq!(a.into_vec(), &[1, 2, 3, 4,])
    }

    #[test]
    fn compact_from_raw_and_leaf_flags() {
        let bytes = vec![0xAB, 0xCD];
        let raw = Nibbles::from_raw(&bytes, false);
        assert_eq!(raw.len(), 4);
        assert!(!raw.is_leaf());
        assert_eq!(raw.to_bytes(), bytes);
        assert_eq!(raw.into_vec(), vec![0xA, 0xB, 0xC, 0xD]);

        let leaf = Nibbles::from_bytes(&bytes);
        assert_eq!(leaf.len(), 4);
        assert!(leaf.is_leaf());
        assert_eq!(leaf.into_vec(), vec![0xA, 0xB, 0xC, 0xD]);

        let empty = Nibbles::from_hex(vec![]);
        assert!(empty.is_empty());
        assert!(!empty.is_leaf());
    }

    #[test]
    fn compact_next_and_choice_update_state() {
        let mut n = Nibbles::from_hex(vec![1, 2, 3]);
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
        let n = Nibbles::from_hex(vec![0xA, 0xB, 0xC, 0xD, 0xE]);
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
        let mut n = Nibbles::from_hex(vec![1, 2, 3]);
        n.prepend(4);
        n.append(5);
        assert_eq!(n.into_vec(), vec![4, 1, 2, 3, 5]);
    }

    #[test]
    fn compact_concat_and_append_new() {
        let a = Nibbles::from_hex(vec![1, 2]);
        let b = Nibbles::from_hex(vec![3, 4]);
        let c = a.concat(&b);
        assert_eq!(c.into_vec(), vec![1, 2, 3, 4]);

        let appended = a.append_new(5);
        assert_eq!(appended.into_vec(), vec![1, 2, 5]);
        assert_eq!(a.into_vec(), vec![1, 2]);
    }

    #[test]
    fn compact_take_clears_self() {
        let mut n = Nibbles::from_raw(&[0xAB, 0xCD], true);
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
        let compact = Nibbles::from_hex(nibbles);
        assert_eq!(compact.encode_compact(), vec![17, 35, 69]);
    }

    #[test]
    fn compact_decode_compact_leaf_roundtrip() {
        let bytes = vec![0xAB, 0xCD];
        let compact = Nibbles::from_raw(&bytes, true);
        let encoded = compact.encode_compact();
        let decoded = Nibbles::decode_compact(&encoded);
        assert_eq!(decoded.to_bytes(), bytes);
        assert!(decoded.is_leaf());
    }

    #[test]
    fn compact_decode_compact_empty() {
        let decoded = Nibbles::decode_compact(&[]);
        assert!(decoded.is_empty());
        assert!(!decoded.is_leaf());
        assert_eq!(decoded.to_bytes(), Vec::<u8>::new());
    }

    #[test]
    fn compact_encode_decode_compact_odd_leaf() {
        let mut compact = Nibbles::from_raw(&[0x12], true);
        compact.append(0x3);
        let encoded = compact.encode_compact();
        let decoded = Nibbles::decode_compact(&encoded);
        assert!(decoded.is_leaf());
        assert_eq!(decoded.into_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn compact_skip_prefix_updates_current() {
        let mut n = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let prefix = Nibbles::from_hex(vec![1, 2, 3]);
        assert!(n.skip_prefix(&prefix));
        assert_eq!(n.current().into_vec(), vec![1, 2, 3]);
        assert_eq!(n.into_vec(), vec![4, 5]);
    }

    #[test]
    fn compact_offset_updates_current() {
        let n = Nibbles::from_hex(vec![1, 2, 3, 4, 5]);
        let offset = n.offset(2);
        assert_eq!(offset.current().into_vec(), vec![1, 2]);
        assert_eq!(offset.into_vec(), vec![3, 4, 5]);
    }

    #[test]
    fn compact_eq_ignores_already_consumed() {
        let a = Nibbles::from_hex(vec![1, 2, 3]);
        let mut b = Nibbles::from_hex(vec![1, 2, 3]);
        b.already_consumed = vec![9, 9];
        assert_eq!(a, b);

        let mut c = Nibbles::from_hex(vec![1, 2, 3]);
        c.is_leaf = true;
        assert_ne!(a, c);
    }

    #[test]
    fn compact_ord_leaf_vs_non_leaf() {
        let non_leaf = Nibbles::from_hex(vec![1, 2]);
        let leaf = Nibbles::from_raw(&[0x12], true);
        assert!(non_leaf < leaf);
    }

    #[test]
    fn compact_ord_leaf_trailing_nibble() {
        let leaf = Nibbles::from_raw(&[0x12], true);
        let non_leaf_long = Nibbles::from_hex(vec![1, 2, 0]);
        assert!(leaf > non_leaf_long);
    }

    #[test]
    fn compact_rlp_encode_empty_nibbles() {
        let nibbles = Nibbles::default();
        let encoded = nibbles.encode_to_vec();
        assert_eq!(encoded, vec![0xc1, 0xc0]);
    }

    #[test]
    fn compact_rlp_encode_decode_nibbles() {
        let mut nibbles = Nibbles::from_hex(vec![0x00, 0x01, 0x02, 0x0f]);
        nibbles.is_leaf = true;
        let encoded = nibbles.encode_to_vec();
        let expected = vec![0xc6, 0xc5, 0x80, 0x01, 0x02, 0x0f, 0x10];
        assert_eq!(encoded, expected);

        let decoded = Nibbles::decode(&encoded).unwrap();
        assert!(decoded.current().is_empty());
        assert_eq!(decoded.clone().into_vec(), vec![0x00, 0x01, 0x02, 0x0f]);
        assert!(decoded.is_leaf());
    }
}
