use std::cmp;

use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

const MAX_NIBBLES: usize = 131;

// TODO: move path-tracking logic somewhere else
// PERF: try using a stack-allocated array
/// Struct representing a list of nibbles (half-bytes)
#[derive(Debug, Copy, Clone)]
pub struct Nibbles {
    data: [u8; MAX_NIBBLES],
    len: u8,
    /// Parts of the path that have already been consumed (used for tracking
    /// current position when visiting nodes). See `current()`.
    consumed: u8,
}

impl Default for Nibbles {
    fn default() -> Self {
        Self {
            data: [0; MAX_NIBBLES],
            len: 0,
            consumed: 0,
        }
    }
}

// NOTE: custom impls to ignore the `already_consumed` field

impl PartialEq for Nibbles {
    fn eq(&self, o: &Self) -> bool {
        self.slice() == o.slice()
    }
}
impl Eq for Nibbles {}

impl PartialOrd for Nibbles {
    fn partial_cmp(&self, o: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Nibbles {
    fn cmp(&self, o: &Self) -> cmp::Ordering {
        self.slice().cmp(o.slice())
    }
}

impl core::hash::Hash for Nibbles {
    fn hash<H: core::hash::Hasher>(&self, h: &mut H) {
        self.slice().hash(h)
    }
}

impl Nibbles {
    /// Create `Nibbles` from  hex-encoded nibbles
    pub fn from_hex(hex: &[u8]) -> Self {
        assert!(hex.len() <= MAX_NIBBLES);
        let mut data = [0u8; MAX_NIBBLES];
        data[..hex.len()].copy_from_slice(hex);
        Self {
            data,
            len: hex.len() as u8,
            consumed: 0,
        }
    }

    /// Splits incoming bytes into nibbles and appends the leaf flag (a 16 nibble at the end)
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_raw(bytes, true)
    }

    /// Splits incoming bytes into nibbles and appends the leaf flag (a 16 nibble at the end) if is_leaf is true
    pub fn from_raw(bytes: &[u8], is_leaf: bool) -> Self {
        let mut data = [0u8; MAX_NIBBLES];
        let mut l = 0usize;

        for &b in bytes {
            data[l] = (b >> 4) & 0x0F;
            l += 1;
            data[l] = b & 0x0F;
            l += 1;
        }
        if is_leaf {
            data[l] = 16;
            l += 1;
        }

        assert!(l <= MAX_NIBBLES);
        Self {
            data,
            len: l as u8,
            consumed: 0,
        }
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.data.to_vec()
    }

    pub fn len(&self) -> usize {
        (self.len - self.consumed) as usize
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn next(&mut self) -> Option<u8> {
        if self.consumed < self.len {
            let b = self.data[self.consumed as usize];
            self.consumed += 1;
            Some(b)
        } else {
            None
        }
    }

    #[inline]
    fn slice(&self) -> &[u8] {
        &self.data[self.consumed as usize..self.len as usize]
    }

    #[inline]
    fn slice_mut(&mut self) -> &mut [u8] {
        &mut self.data[self.consumed as usize..self.len as usize]
    }

    pub fn next_choice(&mut self) -> Option<usize> {
        self.next().filter(|&b| b < 16).map(|b| b as usize)
    }

    pub fn at(&self, i: usize) -> usize {
        self.slice()[i] as usize
    }

    pub fn append(&mut self, nibble: u8) {
        let end = self.len as usize;
        assert!(end < MAX_NIBBLES);
        self.data[end] = nibble;
        self.len += 1;
    }

    pub fn prepend(&mut self, nibble: u8) {
        assert!((self.len as usize) < MAX_NIBBLES);
        // shift slice right by one
        let s = self.slice_mut();
        s.rotate_right(1);
        self.data[self.consumed as usize] = nibble;
        self.len += 1;
    }

    /// If `prefix` is a prefix of self, move the offset after
    /// the prefix and return true, otherwise return false.
    pub fn skip_prefix(&mut self, prefix: &Nibbles) -> bool {
        let p = prefix.as_ref();
        let s = self.slice();

        if s.len() >= p.len() && &s[..p.len()] == p {
            self.consumed += p.len() as u8;
            true
        } else {
            false
        }
    }

    /// Compares self to another, comparing prefixes only in case of unequal lengths.
    pub fn compare_prefix(&self, prefix: &Nibbles) -> cmp::Ordering {
        let a = self.slice();
        let b = prefix.slice();
        let n = a.len().min(b.len());
        a[..n].cmp(&b[..n])
    }

    /// Compares self to another and returns the shared nibble count (amount of nibbles that are equal, from the start)
    pub fn count_prefix(&self, other: &Nibbles) -> usize {
        self.slice()
            .iter()
            .zip(other.slice().iter())
            .take_while(|(a, b)| a == b)
            .count()
    }

    /// Returns the nibbles after the given offset
    pub fn offset(&self, offset: usize) -> Self {
        self.slice_range(offset, self.len())
    }

    /// Returns the nibbles beween the start and end indexes
    pub fn slice_range(&self, start: usize, end: usize) -> Self {
        assert!(start <= end && end <= self.len());
        let mut out = [0u8; MAX_NIBBLES];
        let s = self.slice();
        let n = end - start;
        out[..n].copy_from_slice(&s[start..end]);
        Self {
            data: out,
            len: n as u8,
            consumed: 0,
        }
    }

    /// Extends the nibbles with another list of nibbles
    pub fn extend(&mut self, other: &Nibbles) {
        let this_len = self.len as usize;
        let other_len = other.len();

        // enforce capacity
        assert!(this_len + other_len <= MAX_NIBBLES);

        // copy other’s valid slice into our tail
        let src = other.slice();
        self.data[this_len..this_len + other_len].copy_from_slice(src);

        // update total length
        self.len += other_len as u8;
    }

    /// Taken from https://github.com/citahub/cita_trie/blob/master/src/nibbles.rs#L56
    /// Encodes the nibbles in compact form
    pub fn encode_compact(&self) -> Vec<u8> {
        let s = self.slice(); // active nibble slice
        let is_leaf = self.is_leaf();

        // Trim the leaf flag (value = 16) if present
        let hex = if is_leaf { &s[..s.len() - 1] } else { s };

        let mut compact = Vec::with_capacity(hex.len().div_ceil(2) + 1);

        // Determine prefix nibble according to path length parity
        let mut first: u8;

        if hex.len() & 1 == 1 {
            // odd length → use first nibble as prefix
            first = 0x10 + hex[0]; // prefix high nibble (0x1x)
        } else {
            // even length → prefix = 0x00 or 0x20 for leaf
            first = 0x00;
        }

        if is_leaf {
            first += 0x20; // leaf prefix offset
        }

        compact.push(first);

        // Hex-encode pairs of nibbles into bytes
        let start = if hex.len() & 1 == 1 { 1 } else { 0 };

        for chunk in hex[start..].chunks(2) {
            compact.push((chunk[0] << 4) | chunk[1]);
        }

        compact
    }

    /// Encodes the nibbles in compact form
    pub fn decode_compact(compact: &[u8]) -> Self {
        Self::from_hex(&compact_to_hex(compact))
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
        let a = self.slice();
        let b = other.slice();

        let total = a.len() + b.len();
        assert!(total <= MAX_NIBBLES);

        let mut out = [0u8; MAX_NIBBLES];
        out[..a.len()].copy_from_slice(a);
        out[a.len()..total].copy_from_slice(b);

        Nibbles {
            data: out,
            len: total as u8,
            consumed: 0,
        }
    }

    /// Returns a copy of self with the nibble added at the and
    pub fn append_new(&self, nibble: u8) -> Nibbles {
        let s = self.slice();
        let mut out = [0u8; MAX_NIBBLES];

        let n = s.len();
        assert!(n < MAX_NIBBLES);

        out[..n].copy_from_slice(s);
        out[n] = nibble;

        Nibbles {
            data: out,
            len: (n + 1) as u8,
            consumed: 0,
        }
    }

    /// Return already consumed parts of path
    pub fn current(&self) -> Nibbles {
        let n = self.consumed as usize;

        let mut out = [0u8; MAX_NIBBLES];
        out[..n].copy_from_slice(&self.data[..n]);

        Nibbles {
            data: out,
            len: n as u8,
            consumed: 0,
        }
    }

    /// Empties `self.data` and returns the content
    pub fn take(&mut self) -> Nibbles {
        // length before consuming
        let total = self.len;

        // copy entire valid region into a new buffer
        let mut out = [0u8; MAX_NIBBLES];
        out[..total as usize].copy_from_slice(&self.data[..total as usize]);

        // reset self
        self.len = 0;
        self.consumed = 0;

        Nibbles {
            data: out,
            len: total,
            consumed: 0,
        }
    }
}

impl AsRef<[u8]> for Nibbles {
    fn as_ref(&self) -> &[u8] {
        self.slice()
    }
}

impl RLPEncode for Nibbles {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let slice = self.slice().to_vec();
        Encoder::new(buf).encode_field(&slice).finish();
    }
}

impl RLPDecode for Nibbles {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (data_vec, decoder) = decoder.decode_field::<Vec<u8>>("data")?;

        assert!(data_vec.len() <= MAX_NIBBLES);

        let mut data = [0u8; MAX_NIBBLES];
        data[..data_vec.len()].copy_from_slice(&data_vec);

        let out = Nibbles {
            data,
            len: data_vec.len() as u8,
            consumed: 0,
        };

        Ok((out, decoder.finish()?))
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
        let mut a = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(&[1, 2, 3]);
        assert!(a.skip_prefix(&b));
        assert_eq!(a.as_ref(), &[4, 5])
    }

    #[test]
    fn skip_prefix_true_same_length() {
        let mut a = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        assert!(a.skip_prefix(&b));
        assert!(a.is_empty());
    }

    #[test]
    fn skip_prefix_longer_prefix() {
        let mut a = Nibbles::from_hex(&[1, 2, 3]);
        let b = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        assert!(!a.skip_prefix(&b));
        assert_eq!(a.as_ref(), &[1, 2, 3])
    }

    #[test]
    fn skip_prefix_false() {
        let mut a = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(&[1, 2, 4]);
        assert!(!a.skip_prefix(&b));
        assert_eq!(a.as_ref(), &[1, 2, 3, 4, 5])
    }

    #[test]
    fn count_prefix_all() {
        let a = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        assert_eq!(a.count_prefix(&b), a.len());
    }

    #[test]
    fn count_prefix_partial() {
        let a = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(&[1, 2, 3]);
        assert_eq!(a.count_prefix(&b), b.len());
    }

    #[test]
    fn count_prefix_none() {
        let a = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(&[2, 3, 4, 5, 6]);
        assert_eq!(a.count_prefix(&b), 0);
    }

    #[test]
    fn compare_prefix_equal() {
        let a = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Equal);
    }

    #[test]
    fn compare_prefix_less() {
        let a = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(&[1, 2, 4, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Less);
    }

    #[test]
    fn compare_prefix_greater() {
        let a = Nibbles::from_hex(&[1, 2, 4, 4, 5]);
        let b = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Greater);
    }

    #[test]
    fn compare_prefix_equal_b_longer() {
        let a = Nibbles::from_hex(&[1, 2, 3]);
        let b = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        assert_eq!(a.compare_prefix(&b), Ordering::Equal);
    }

    #[test]
    fn compare_prefix_equal_a_longer() {
        let a = Nibbles::from_hex(&[1, 2, 3, 4, 5]);
        let b = Nibbles::from_hex(&[1, 2, 3]);
        assert_eq!(a.compare_prefix(&b), Ordering::Equal);
    }
}
