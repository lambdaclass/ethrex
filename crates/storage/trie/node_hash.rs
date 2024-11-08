use ethereum_rust_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethereum_types::H256;
#[cfg(feature = "libmdbx")]
use libmdbx::orm::{Decodable, Encodable};
use sha3::{Digest, Keccak256};

/// Struct representing a trie node hash
/// If the encoded node is less than 32 bits, contains the encoded node itself
// TODO: Check if we can omit the Inline variant, as nodes will always be bigger than 32 bits in our use case
// TODO: Check if making this `Copy` can make the code less verbose at a reasonable performance cost
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeHash {
    Hashed(H256),
    Inline(Vec<u8>),
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
