use std::{cmp, mem};

use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

// TODO: move path-tracking logic somewhere else
/// Struct representing a list of nibbles (half-bytes)
///
/// `data` contains ALL nibbles (both consumed and unconsumed).
/// `consumed` is the index marking the boundary: `data[..consumed]` are
/// consumed nibbles (the traversal path so far), `data[consumed..]` are
/// the remaining unconsumed nibbles.
///
/// This design makes `next()` O(1) (just increment `consumed`) instead of
/// O(n) (`Vec::remove(0)` which shifts all elements).
///
/// Invariant: Nibbles stored in trie nodes (LeafNode.partial, ExtensionNode.prefix)
/// always have `consumed == 0` because they are created via `offset()` or `slice()`
/// which produce compacted Nibbles.
#[derive(
    Debug,
    Clone,
    Default,
    rkyv::Serialize,
    rkyv::Deserialize,
    rkyv::Archive,
)]
pub struct Nibbles {
    data: Vec<u8>,
    /// Number of nibbles consumed from the front.
    /// Skipped in rkyv serialization — defaults to 0 on deserialization.
    /// This is safe because Nibbles stored in nodes always have consumed=0.
    #[rkyv(with = rkyv::with::Skip)]
    consumed: usize,
}

// Custom serde: serialize only the effective (unconsumed) nibbles.
// This matches the old format where `data` only contained unconsumed nibbles.
impl serde::Serialize for Nibbles {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("Nibbles", 1)?;
        s.serialize_field("data", &self.data[self.consumed..])?;
        s.end()
    }
}

impl<'de> serde::Deserialize<'de> for Nibbles {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        struct NibblesVisitor;

        impl<'de> Visitor<'de> for NibblesVisitor {
            type Value = Nibbles;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct Nibbles")
            }

            fn visit_map<V: MapAccess<'de>>(self, mut map: V) -> Result<Nibbles, V::Error> {
                let mut data: Option<Vec<u8>> = None;
                while let Some(key) = map.next_key::<&str>()? {
                    match key {
                        "data" => data = Some(map.next_value()?),
                        // Silently ignore unknown fields (e.g. old "already_consumed")
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(Nibbles {
                    data: data.unwrap_or_default(),
                    consumed: 0,
                })
            }
        }

        deserializer.deserialize_struct("Nibbles", &["data"], NibblesVisitor)
    }
}

// NOTE: custom impls to compare only effective (unconsumed) nibbles

impl PartialEq for Nibbles {
    fn eq(&self, other: &Nibbles) -> bool {
        self.data[self.consumed..] == other.data[other.consumed..]
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
        self.data[self.consumed..].cmp(&other.data[other.consumed..])
    }
}

impl std::hash::Hash for Nibbles {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.data[self.consumed..].hash(state);
    }
}

impl Nibbles {
    /// Create `Nibbles` from hex-encoded nibbles
    pub const fn from_hex(hex: Vec<u8>) -> Self {
        Self {
            data: hex,
            consumed: 0,
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

        Self { data, consumed: 0 }
    }

    pub fn into_vec(mut self) -> Vec<u8> {
        if self.consumed > 0 {
            self.data.drain(..self.consumed);
        }
        self.data
    }

    /// Returns the amount of effective (unconsumed) nibbles
    pub fn len(&self) -> usize {
        self.data.len() - self.consumed
    }

    /// Returns true if there are no effective nibbles
    pub fn is_empty(&self) -> bool {
        self.consumed >= self.data.len()
    }

    /// If `prefix` is a prefix of self's effective nibbles, advance past it
    /// and return true, otherwise return false. O(1) for the advance.
    pub fn skip_prefix(&mut self, prefix: &Nibbles) -> bool {
        let effective = &self.data[self.consumed..];
        let prefix_data = prefix.as_ref();
        if effective.len() >= prefix_data.len() && effective[..prefix_data.len()] == *prefix_data {
            self.consumed += prefix_data.len();
            true
        } else {
            false
        }
    }

    /// Compares self to another, comparing prefixes only in case of unequal lengths.
    pub fn compare_prefix(&self, prefix: &Nibbles) -> cmp::Ordering {
        let effective = &self.data[self.consumed..];
        let prefix_data = prefix.as_ref();
        if effective.len() > prefix_data.len() {
            effective[..prefix_data.len()].cmp(prefix_data)
        } else {
            effective.cmp(&prefix_data[..effective.len()])
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

    /// Consumes and returns the first effective nibble. O(1).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<u8> {
        if self.consumed < self.data.len() {
            let val = self.data[self.consumed];
            self.consumed += 1;
            Some(val)
        } else {
            None
        }
    }

    /// Consumes and returns the first effective nibble if it is a suitable choice index (aka < 16)
    pub fn next_choice(&mut self) -> Option<usize> {
        self.next().filter(|choice| *choice < 16).map(usize::from)
    }

    /// Returns a compacted Nibbles containing the effective nibbles starting at
    /// `offset` from the current position. The returned Nibbles has consumed=0,
    /// suitable for node storage (partial/prefix). Does NOT preserve consumed
    /// tracking — use `advance()` for traversal where `current()` is needed.
    pub fn offset(&self, offset: usize) -> Nibbles {
        Nibbles {
            data: self.data[self.consumed + offset..].to_vec(),
            consumed: 0,
        }
    }

    /// Advances past `n` nibbles while preserving consumed tracking.
    /// Use this during trie traversal when `current()` will be called later.
    pub fn advance(&self, n: usize) -> Nibbles {
        Nibbles {
            data: self.data.clone(),
            consumed: self.consumed + n,
        }
    }

    /// Returns a compacted Nibbles between the start and end indexes
    /// (relative to effective position). The returned Nibbles has consumed=0.
    pub fn slice(&self, start: usize, end: usize) -> Nibbles {
        Nibbles::from_hex(self.data[self.consumed + start..self.consumed + end].to_vec())
    }

    /// Extends the nibbles with another list of nibbles
    pub fn extend(&mut self, other: &Nibbles) {
        self.data.extend_from_slice(other.as_ref());
    }

    /// Return the nibble at the given index (relative to effective position),
    /// will panic if the index is out of range
    pub fn at(&self, i: usize) -> usize {
        self.data[self.consumed + i] as usize
    }

    /// Inserts a nibble at the start of the effective nibbles
    pub fn prepend(&mut self, nibble: u8) {
        if self.consumed > 0 {
            // Reuse a consumed slot — O(1)
            self.consumed -= 1;
            self.data[self.consumed] = nibble;
        } else {
            // Fallback: shift data right — O(n), but rare in practice
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
        let effective = &self.data[self.consumed..];
        let mut compact = vec![];
        let is_leaf = self.is_leaf();
        let mut hex = if is_leaf {
            &effective[..effective.len() - 1]
        } else {
            effective
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
        if self.is_empty() {
            false
        } else {
            self.data[self.data.len() - 1] == 16
        }
    }

    /// Combines the effective nibbles into bytes, trimming the leaf flag if necessary
    pub fn to_bytes(&self) -> Vec<u8> {
        let effective = &self.data[self.consumed..];
        // Trim leaf flag
        let data = if !effective.is_empty() && *effective.last().unwrap() == 16 {
            &effective[..effective.len() - 1]
        } else {
            effective
        };
        // Combine nibbles into bytes
        data.chunks(2)
            .map(|chunk| match chunk.len() {
                1 => chunk[0] << 4,
                _ => chunk[0] << 4 | chunk[1],
            })
            .collect::<Vec<_>>()
    }

    /// Concatenates self and another Nibbles returning a new Nibbles.
    /// Preserves the full data (consumed + unconsumed) of self.
    pub fn concat(&self, other: &Nibbles) -> Nibbles {
        let mut data = self.data.clone();
        data.extend_from_slice(other.as_ref());
        Nibbles {
            data,
            consumed: self.consumed,
        }
    }

    /// Returns a copy of self with the nibble added at the end
    pub fn append_new(&self, nibble: u8) -> Nibbles {
        let mut data = self.data.clone();
        data.push(nibble);
        Nibbles {
            data,
            consumed: self.consumed,
        }
    }

    /// Return already consumed parts of path as a new Nibbles
    pub fn current(&self) -> Nibbles {
        Nibbles {
            data: self.data[..self.consumed].to_vec(),
            consumed: 0,
        }
    }

    /// Empties `self` and returns the old content
    pub fn take(&mut self) -> Self {
        Nibbles {
            data: mem::take(&mut self.data),
            consumed: mem::replace(&mut self.consumed, 0),
        }
    }
}

impl AsRef<[u8]> for Nibbles {
    fn as_ref(&self) -> &[u8] {
        &self.data[self.consumed..]
    }
}

impl RLPEncode for Nibbles {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        // Encode only the effective (unconsumed) nibbles.
        // The to_vec() allocation here is acceptable since encode is only called
        // during commit/serialization, not on the insert hot path.
        let effective = self.data[self.consumed..].to_vec();
        Encoder::new(buf).encode_field(&effective).finish();
    }
}

impl RLPDecode for Nibbles {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (data, decoder) = decoder.decode_field("data")?;
        Ok((Self { data, consumed: 0 }, decoder.finish()?))
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
