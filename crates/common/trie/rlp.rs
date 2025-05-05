// Contains RLP encoding and decoding implementations for Trie Nodes
// This encoding is only used to store the nodes in the DB, it is not the encoding used for hash computation
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

use super::node::{BranchNode, ExtensionNode, LeafNode, Node};
use crate::{
    cache::CacheKey, node_hash::NodeHash, Nibbles, PathRLP, TrieState, ValueRLP, EMPTY_TRIE_HASH,
};

enum NodeType {
    Branch = 0,
    Extension = 1,
    Leaf = 2,
}

impl NodeType {
    fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(Self::Branch),
            1 => Some(Self::Extension),
            2 => Some(Self::Leaf),
            _ => None,
        }
    }
}

pub struct BranchNodeEncoder<'a>(pub &'a BranchNode, pub &'a TrieState);
impl<'a> RLPEncode for BranchNodeEncoder<'a> {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let choices = self.0.choices.map(|key| {
            key.is_valid()
                .then(|| self.1[key].compute_hash(self.1))
                .unwrap_or_default()
        });

        // TODO: choices encoded as vec due to conflicting trait impls for [T;N] & [u8;N], check if we can fix this later
        Encoder::new(buf)
            .encode_field(&choices.to_vec())
            .encode_field(&self.0.value)
            .finish()
    }
}

pub struct ExtensionNodeEncoder<'a>(pub &'a ExtensionNode, pub &'a TrieState);
impl<'a> RLPEncode for ExtensionNodeEncoder<'a> {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let child = self
            .0
            .child
            .is_valid()
            .then(|| self.1[self.0.child].compute_hash(self.1))
            .unwrap_or_default();

        Encoder::new(buf)
            .encode_field(&self.0.prefix)
            .encode_field(&child)
            .finish()
    }
}

pub struct LeafNodeEncoder<'a>(pub &'a LeafNode, pub &'a TrieState);
impl<'a> RLPEncode for LeafNodeEncoder<'a> {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.0.partial)
            .encode_field(&self.0.value)
            .finish()
    }
}

pub struct BranchNodeDecoder {
    choices: Vec<NodeHash>,
    value: ValueRLP,
}
impl BranchNodeDecoder {
    pub fn finish(self, path: Nibbles, state: &mut TrieState) -> CacheKey {
        let mut choices = [CacheKey::INVALID; 16];
        for (nibble, choice) in self.choices.into_iter().enumerate() {
            if choice.is_valid() {
                // TODO: Is encode_compact() the correct method?
                choices[nibble] = state
                    .cache
                    .insert(path.append_new(nibble as u8).encode_compact(), choice)
                    .0;
            }
        }

        state
            .cache
            .insert(
                // TODO: Is encode_compact() the correct method?
                path.encode_compact(),
                Node::from(BranchNode {
                    choices,
                    value: self.value,
                }),
            )
            .0
    }
}
impl RLPDecode for BranchNodeDecoder {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        const CHOICES_LEN_ERROR_MSG: &str =
            "Error decoding field 'choices' of type [H256;16]: Invalid Length";
        let decoder = Decoder::new(rlp)?;
        let (choices, decoder) = decoder.decode_field::<Vec<NodeHash>>("choices")?;
        let choices = choices
            .try_into()
            .map_err(|_| RLPDecodeError::Custom(CHOICES_LEN_ERROR_MSG.to_string()))?;
        let (value, decoder) = decoder.decode_field("value")?;
        Ok((Self { choices, value }, decoder.finish()?))
    }
}

pub struct ExtensionNodeDecoder {
    prefix: Nibbles,
    child: NodeHash,
}
impl ExtensionNodeDecoder {
    pub fn finish(self, path: Nibbles, state: &mut TrieState) -> CacheKey {
        let child = state
            .cache
            .insert(
                // TODO: Is encode_compact() the correct method?
                path.concat(self.prefix.clone()).encode_compact(),
                self.child,
            )
            .0;

        state
            .cache
            .insert(
                // TODO: Is encode_compact() the correct method?
                path.encode_compact(),
                Node::from(ExtensionNode {
                    prefix: self.prefix,
                    child,
                }),
            )
            .0
    }
}
impl RLPDecode for ExtensionNodeDecoder {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (prefix, decoder) = decoder.decode_field("prefix")?;
        let (child, decoder) = decoder.decode_field("child")?;
        Ok((Self { prefix, child }, decoder.finish()?))
    }
}

pub struct LeafNodeDecoder {
    partial: Nibbles,
    value: ValueRLP,
}
impl LeafNodeDecoder {
    pub fn finish(self, path: Nibbles, state: &mut TrieState) -> CacheKey {
        state
            .cache
            .insert(
                // TODO: Is encode_compact() the correct method?
                path.encode_compact(),
                Node::from(LeafNode {
                    partial: path.concat(self.partial),
                    value: self.value,
                }),
            )
            .0
    }
}
impl RLPDecode for LeafNodeDecoder {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (partial, decoder) = decoder.decode_field("partial")?;
        let (value, decoder) = decoder.decode_field("value")?;
        Ok((Self { partial, value }, decoder.finish()?))
    }
}

pub struct NodeEncoder<'a>(pub &'a Node, pub &'a TrieState);
impl<'a> RLPEncode for NodeEncoder<'a> {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let node_type = match self.0 {
            Node::Branch(_) => NodeType::Branch,
            Node::Extension(_) => NodeType::Extension,
            Node::Leaf(_) => NodeType::Leaf,
        };
        buf.put_u8(node_type as u8);
        match self.0 {
            Node::Branch(n) => BranchNodeEncoder(n, self.1).encode(buf),
            Node::Extension(n) => ExtensionNodeEncoder(n, self.1).encode(buf),
            Node::Leaf(n) => LeafNodeEncoder(n, self.1).encode(buf),
        }
    }
}

pub enum NodeDecoder {
    Branch(BranchNodeDecoder),
    Extension(ExtensionNodeDecoder),
    Leaf(LeafNodeDecoder),
}
impl NodeDecoder {
    pub fn finish(self, path: Nibbles, state: &mut TrieState) -> CacheKey {
        todo!()
    }
}
impl RLPDecode for NodeDecoder {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let node_type = rlp.first().ok_or(RLPDecodeError::InvalidLength)?;
        let node_type = NodeType::from_u8(*node_type).ok_or(RLPDecodeError::MalformedData)?;
        let rlp = &rlp[1..];
        match node_type {
            NodeType::Branch => BranchNodeDecoder::decode_unfinished(rlp)
                .map(|(node, rem)| (Self::Branch(node), rem)),
            NodeType::Extension => ExtensionNodeDecoder::decode_unfinished(rlp)
                .map(|(node, rem)| (Self::Extension(node), rem)),
            NodeType::Leaf => {
                LeafNodeDecoder::decode_unfinished(rlp).map(|(node, rem)| (Self::Leaf(node), rem))
            }
        }
    }
}
