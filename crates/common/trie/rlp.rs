use std::array;

// Contains RLP encoding and decoding implementations for Trie Nodes
// This encoding is only used to store the nodes in the DB, it is not the encoding used for hash computation
use ethrex_rlp::{
    decode::{RLPDecode, decode_bytes},
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

use super::node::{BranchNode, ExtensionNode, LeafNode, Node};
use crate::{Nibbles, NodeHash};

impl RLPEncode for BranchNode {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let mut encoder = Encoder::new(buf);
        for child in self.choices.iter() {
            match child.compute_hash() {
                NodeHash::Hashed(hash) => encoder = encoder.encode_bytes(&hash.0),
                child @ NodeHash::Inline(raw) if raw.1 != 0 => {
                    encoder = encoder.encode_raw(child.as_ref())
                }
                _ => encoder = encoder.encode_bytes(&[]),
            }
        }
        encoder = encoder.encode_bytes(&self.value);
        encoder.finish();
    }
}

impl RLPEncode for ExtensionNode {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let mut encoder = Encoder::new(buf).encode_bytes(&self.prefix.encode_compact());
        encoder = self.child.compute_hash().encode(encoder);
        encoder.finish();
    }
}

impl RLPEncode for LeafNode {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_bytes(&self.partial.encode_compact())
            .encode_bytes(&self.value)
            .finish()
    }
}

impl RLPEncode for Node {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        match self {
            Node::Branch(n) => n.encode(buf),
            Node::Extension(n) => n.encode(buf),
            Node::Leaf(n) => n.encode(buf),
        }
    }
}

impl RLPDecode for Node {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let mut rlp_items = vec![];
        let mut decoder = Decoder::new(rlp)?;
        let mut item;
        // Get encoded fields
        loop {
            (item, decoder) = decoder.get_encoded_item()?;
            rlp_items.push(item);
            // Check if we reached the end or if we decoded more items than the ones we need
            if decoder.is_done() || rlp_items.len() > 17 {
                break;
            }
        }
        // Deserialize into node depending on the available fields
        Ok((
            match rlp_items.len() {
                // Leaf or Extension Node
                2 => {
                    let (path, _) = decode_bytes(&rlp_items[0])?;
                    let path = Nibbles::decode_compact(path);
                    if path.is_leaf() {
                        // Decode as Leaf
                        let (value, _) = decode_bytes(&rlp_items[1])?;
                        LeafNode {
                            partial: path,
                            value: value.to_vec(),
                        }
                        .into()
                    } else {
                        // Decode as Extension
                        ExtensionNode {
                            prefix: path,
                            child: decode_child(&rlp_items[1]).into(),
                        }
                        .into()
                    }
                }
                // Branch Node
                17 => {
                    let choices = array::from_fn(|i| decode_child(&rlp_items[i]).into());
                    let (value, _) = decode_bytes(&rlp_items[16])?;
                    BranchNode {
                        choices,
                        value: value.to_vec(),
                    }
                    .into()
                }
                n => {
                    return Err(RLPDecodeError::Custom(format!(
                        "Invalid arg count for Node, expected 2 or 17, got {n}"
                    )));
                }
            },
            decoder.finish()?,
        ))
    }
}

fn decode_child(rlp: &[u8]) -> NodeHash {
    match decode_bytes(rlp) {
        Ok((hash, &[])) if hash.len() == 32 => NodeHash::from_slice(hash),
        Ok((&[], &[])) => NodeHash::default(),
        _ => NodeHash::from_slice(rlp),
    }
}
