use ethereum_rust_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethereum_types::H256;
#[cfg(feature = "libmdbx")]
use libmdbx::orm::{Decodable, Encodable};
use sha3::{Digest, Keccak256};

use super::nibble::{NibbleSlice, NibbleVec};

/// Struct representing a trie node hash
/// If the encoded node is less than 32 bits, contains the encoded node itself
// TODO: Check if we can omit the Inline variant, as nodes will always be bigger than 32 bits in our use case
// TODO: Check if making this `Copy` can make the code less verbose at a reasonable performance cost
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeHash {
    Hashed(H256),
    Inline(Vec<u8>),
}

const fn compute_byte_usage(value: usize) -> usize {
    let bits_used = usize::BITS as usize - value.leading_zeros() as usize;
    (bits_used.saturating_sub(1) >> 3) + 1
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PathKind {
    Extension,
    Leaf,
}

impl PathKind {
    const fn into_flag(self) -> u8 {
        match self {
            PathKind::Extension => 0x00,
            PathKind::Leaf => 0x20,
        }
    }
}

impl AsRef<[u8]> for NodeHash {
    fn as_ref(&self) -> &[u8] {
        match self {
            NodeHash::Inline(x) => x.as_ref(),
            NodeHash::Hashed(x) => x.as_bytes(),
        }
    }
}

impl NodeHash {
    /// Returns the `NodeHash` of an encoded node (encoded using the NodeEncoder)
    pub fn from_encoded_raw(encoded: Vec<u8>) -> NodeHash {
        if encoded.len() >= 32 {
            let hash = Keccak256::new_with_prefix(&encoded).finalize();
            NodeHash::Hashed(H256::from_slice(hash.as_slice()))
        } else {
            NodeHash::Inline(encoded)
        }
    }
    /// Returns the finalized hash
    /// NOTE: This will hash smaller nodes, only use to get the final root hash, not for intermediate node hashes
    pub fn finalize(self) -> H256 {
        match self {
            NodeHash::Inline(x) => {
                H256::from_slice(Keccak256::new().chain_update(&*x).finalize().as_slice())
            }
            NodeHash::Hashed(x) => x,
        }
    }

    /// Returns true if the hash is valid
    /// The hash will only be considered invalid if it is empty
    /// Aka if it has a default value instead of being a product of hash computation
    pub fn is_valid(&self) -> bool {
        !matches!(self, NodeHash::Inline(v) if v.is_empty())
    }

    /// Const version of `Default` trait impl
    pub const fn const_default() -> Self {
        Self::Inline(vec![])
    }
}

impl From<Vec<u8>> for NodeHash {
    fn from(value: Vec<u8>) -> Self {
        match value.len() {
            32 => NodeHash::Hashed(H256::from_slice(&value)),
            _ => NodeHash::Inline(value),
        }
    }
}

impl From<H256> for NodeHash {
    fn from(value: H256) -> Self {
        NodeHash::Hashed(value)
    }
}

impl From<NodeHash> for Vec<u8> {
    fn from(val: NodeHash) -> Self {
        match val {
            NodeHash::Hashed(x) => x.0.to_vec(),
            NodeHash::Inline(x) => x,
        }
    }
}

impl From<&NodeHash> for Vec<u8> {
    fn from(val: &NodeHash) -> Self {
        match val {
            NodeHash::Hashed(x) => x.0.to_vec(),
            NodeHash::Inline(x) => x.clone(),
        }
    }
}

#[cfg(feature = "libmdbx")]
impl Encodable for NodeHash {
    type Encoded = Vec<u8>;

    fn encode(self) -> Self::Encoded {
        self.into()
    }
}

#[cfg(feature = "libmdbx")]
impl Decodable for NodeHash {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        Ok(match b.len() {
            32 => NodeHash::Hashed(H256::from_slice(b)),
            _ => NodeHash::Inline(b.into()),
        })
    }
}

impl Default for NodeHash {
    fn default() -> Self {
        NodeHash::Inline(Vec::new())
    }
}

// Encoded as Vec<u8>
impl RLPEncode for NodeHash {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        RLPEncode::encode(&Into::<Vec<u8>>::into(self), buf)
    }
}

impl RLPDecode for NodeHash {
    fn decode_unfinished(
        rlp: &[u8],
    ) -> Result<(Self, &[u8]), ethereum_rust_rlp::error::RLPDecodeError> {
        let (hash, rest): (Vec<u8>, &[u8]);
        (hash, rest) = RLPDecode::decode_unfinished(rlp)?;
        let hash = NodeHash::from(hash);
        Ok((hash, rest))
    }
}

#[derive(Default, Debug)]
pub struct NodeEncoder {
    encoded: Vec<u8>,
}

impl NodeEncoder {
    pub fn new() -> Self {
        Self {
            ..Default::default()
        }
    }

    pub const fn path_len(value_len: usize) -> usize {
        Self::bytes_len((value_len >> 1) + 1, 0)
    }

    pub const fn bytes_len(value_len: usize, first_value: u8) -> usize {
        match value_len {
            1 if first_value < 128 => 1,
            l if l < 56 => l + 1,
            l => l + compute_byte_usage(l) + 1,
        }
    }

    pub fn write_list_header(&mut self, children_len: usize) {
        self.write_len(0xC0, 0xF7, children_len);
    }

    fn write_len(&mut self, short_base: u8, long_base: u8, value: usize) {
        match value {
            l if l < 56 => self.write_raw(&[short_base + l as u8]),
            l => {
                let l_len = compute_byte_usage(l);
                self.write_raw(&[long_base + l_len as u8]);
                self.write_raw(&l.to_be_bytes()[size_of::<usize>() - l_len..]);
            }
        }
    }

    pub fn write_raw(&mut self, value: &[u8]) {
        self.encoded.extend_from_slice(value);
    }

    pub fn write_path_slice(&mut self, value: &NibbleSlice, kind: PathKind) {
        let mut flag = kind.into_flag();

        // TODO: Do not use iterators.
        let nibble_count = value.clone().count();
        let nibble_iter = if nibble_count & 0x01 != 0 {
            let mut iter = value.clone();
            flag |= 0x10;
            flag |= iter.next().unwrap() as u8;
            iter
        } else {
            value.clone()
        };

        let i2 = nibble_iter.clone().skip(1).step_by(2);
        if nibble_count > 1 {
            self.write_len(0x80, 0xB7, (nibble_count >> 1) + 1);
        }
        self.write_raw(&[flag]);
        for (a, b) in nibble_iter.step_by(2).zip(i2) {
            self.write_raw(&[((a as u8) << 4) | (b as u8)]);
        }
    }

    pub fn write_path_vec(&mut self, value: &NibbleVec, kind: PathKind) {
        let mut flag = kind.into_flag();

        // TODO: Do not use iterators.
        let nibble_count = value.len();
        let nibble_iter = if nibble_count & 0x01 != 0 {
            let mut iter = value.iter();
            flag |= 0x10;
            flag |= iter.next().unwrap() as u8;
            iter
        } else {
            value.iter()
        };

        let i2 = nibble_iter.clone().skip(1).step_by(2);
        if nibble_count > 1 {
            self.write_len(0x80, 0xB7, (nibble_count >> 1) + 1);
        }
        self.write_raw(&[flag]);
        for (a, b) in nibble_iter.step_by(2).zip(i2) {
            self.write_raw(&[((a as u8) << 4) | (b as u8)]);
        }
    }

    pub fn write_bytes(&mut self, value: &[u8]) {
        if value.len() == 1 && value[0] < 128 {
            self.write_raw(&[value[0]]);
        } else {
            self.write_len(0x80, 0xB7, value.len());
            self.write_raw(value);
        }
    }

    pub fn finalize(self) -> Vec<u8> {
        self.encoded
    }
}
