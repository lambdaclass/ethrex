use std::array;

// Contains RLP encoding and decoding implementations for Trie Nodes
// This encoding is only used to store the nodes in the DB, it is not the encoding used for hash computation
use librlp::{Header, RlpBuf, RlpDecode, RlpEncode, RlpError, encode::write_list_header};

const RLP_NULL: u8 = 0x80;

use super::node::{BranchNode, ExtensionNode, LeafNode, Node};
use crate::{Nibbles, NodeHash};

impl RlpEncode for BranchNode {
    fn encode(&self, buf: &mut RlpBuf) {
        buf.list(|buf| {
            for child in self.choices.iter() {
                match child.compute_hash_ref() {
                    NodeHash::Hashed(hash) => hash.0.encode(buf),
                    NodeHash::Inline((_, 0)) => buf.put_u8(RLP_NULL),
                    NodeHash::Inline((encoded, len)) => {
                        buf.put_bytes(&encoded[..*len as usize])
                    }
                }
            }
            self.value.as_slice().encode(buf);
        });
    }

    fn encoded_length(&self) -> usize {
        let value_len = self.value.as_slice().encoded_length();
        let payload_len = self.choices.iter().fold(value_len, |acc, child| {
            acc + RlpEncode::encoded_length(child.compute_hash_ref())
        });
        Header::list_header_len(payload_len) + payload_len
    }

    // Duplicated to prealloc the buffer and avoid calculating the payload length twice
    fn encode_to_vec(&self, out: &mut Vec<u8>) {
        let value_len = self.value.as_slice().encoded_length();
        let choices_len = self.choices.iter().fold(0, |acc, child| {
            acc + RlpEncode::encoded_length(child.compute_hash_ref())
        });
        let payload_len = choices_len + value_len;

        out.reserve(payload_len + 3); // 3 byte prefix headroom

        write_list_header(payload_len, out);
        for child in self.choices.iter() {
            match child.compute_hash_ref() {
                NodeHash::Hashed(hash) => hash.0.encode_to_vec(out),
                NodeHash::Inline((_, 0)) => out.push(RLP_NULL),
                NodeHash::Inline((encoded, len)) => {
                    out.extend_from_slice(&encoded[..*len as usize])
                }
            }
        }
        self.value.as_slice().encode_to_vec(out);
    }
}

impl RlpEncode for ExtensionNode {
    fn encode(&self, buf: &mut RlpBuf) {
        buf.list(|buf| {
            self.prefix.encode_compact().as_slice().encode(buf);
            match self.child.compute_hash_ref() {
                NodeHash::Hashed(hash) => hash.0.encode(buf),
                NodeHash::Inline((encoded, len)) => {
                    buf.put_bytes(&encoded[..*len as usize])
                }
            }
        });
    }

    fn encoded_length(&self) -> usize {
        let prefix_len = self.prefix.encode_compact().as_slice().encoded_length();
        let child_len = RlpEncode::encoded_length(self.child.compute_hash_ref());
        let payload_len = prefix_len + child_len;
        Header::list_header_len(payload_len) + payload_len
    }
}

impl RlpEncode for LeafNode {
    fn encode(&self, buf: &mut RlpBuf) {
        buf.list(|buf| {
            self.partial.encode_compact().as_slice().encode(buf);
            self.value.as_slice().encode(buf);
        });
    }

    fn encoded_length(&self) -> usize {
        let prefix_len = self.partial.encode_compact().as_slice().encoded_length();
        let value_len = self.value.as_slice().encoded_length();
        let payload_len = prefix_len + value_len;
        Header::list_header_len(payload_len) + payload_len
    }
}

impl RlpEncode for Node {
    fn encode(&self, buf: &mut RlpBuf) {
        match self {
            Node::Branch(n) => n.encode(buf),
            Node::Extension(n) => n.encode(buf),
            Node::Leaf(n) => n.encode(buf),
        }
    }

    fn encoded_length(&self) -> usize {
        match self {
            Node::Branch(n) => n.encoded_length(),
            Node::Extension(n) => n.encoded_length(),
            Node::Leaf(n) => n.encoded_length(),
        }
    }
}

impl RlpDecode for Node {
    fn decode(buf: &mut &[u8]) -> Result<Self, RlpError> {
        let header = Header::decode(buf)?;
        if !header.list {
            return Err(RlpError::UnexpectedString);
        }

        let payload = &buf[..header.payload_length];
        *buf = &buf[header.payload_length..];

        // Count items in the list payload
        let mut rlp_items: [Option<&[u8]>; 17] = Default::default();
        let mut rlp_items_len = 0;
        let mut remaining = payload;

        while !remaining.is_empty() && rlp_items_len < 17 {
            let item_start = remaining;
            // Peek at the first byte to determine item boundaries
            let first = remaining[0];
            let item_len = if first < 0x80 {
                // Single byte
                1
            } else if first <= 0xb7 {
                // Short string: 0x80..=0xb7
                1 + (first - 0x80) as usize
            } else if first <= 0xbf {
                // Long string
                let len_of_len = (first - 0xb7) as usize;
                let mut len = 0usize;
                for i in 0..len_of_len {
                    len = len << 8 | remaining[1 + i] as usize;
                }
                1 + len_of_len + len
            } else if first <= 0xf7 {
                // Short list: 0xc0..=0xf7
                1 + (first - 0xc0) as usize
            } else {
                // Long list
                let len_of_len = (first - 0xf7) as usize;
                let mut len = 0usize;
                for i in 0..len_of_len {
                    len = len << 8 | remaining[1 + i] as usize;
                }
                1 + len_of_len + len
            };

            if item_len > remaining.len() {
                return Err(RlpError::InputTooShort);
            }

            rlp_items[rlp_items_len] = Some(&item_start[..item_len]);
            remaining = &remaining[item_len..];
            rlp_items_len += 1;
        }

        if !remaining.is_empty() {
            return Err(RlpError::Custom(
                "Invalid arg count for Node, expected 2 or 17, got more than 17".to_string(),
            ));
        }

        // Deserialize into node depending on the available fields
        let node = match rlp_items_len {
            // Leaf or Extension Node
            2 => {
                let path_rlp = rlp_items[0].expect("we already checked the length");
                let mut path_buf: &[u8] = path_rlp;
                let path_bytes: Vec<u8> = RlpDecode::decode(&mut path_buf)?;
                let path = Nibbles::decode_compact(&path_bytes);
                if path.is_leaf() {
                    // Decode as Leaf
                    let value_rlp = rlp_items[1].expect("we already checked the length");
                    let mut value_buf: &[u8] = value_rlp;
                    let value: Vec<u8> = RlpDecode::decode(&mut value_buf)?;
                    LeafNode {
                        partial: path,
                        value,
                    }
                    .into()
                } else {
                    // Decode as Extension
                    ExtensionNode {
                        prefix: path,
                        child: decode_child(rlp_items[1].expect("we already checked the length"))
                            .into(),
                    }
                    .into()
                }
            }
            // Branch Node
            17 => {
                let choices = array::from_fn(|i| {
                    decode_child(rlp_items[i].expect("we already checked the length")).into()
                });
                let value_rlp = rlp_items[16].expect("we already checked the length");
                let mut value_buf: &[u8] = value_rlp;
                let value: Vec<u8> = RlpDecode::decode(&mut value_buf)?;
                BranchNode { choices, value }.into()
            }
            n => {
                return Err(RlpError::Custom(format!(
                    "Invalid arg count for Node, expected 2 or 17, got {n}"
                )));
            }
        };
        Ok(node)
    }
}

fn decode_child(rlp: &[u8]) -> NodeHash {
    let mut buf: &[u8] = rlp;
    match Vec::<u8>::decode(&mut buf) {
        Ok(hash) if buf.is_empty() && hash.len() == 32 => NodeHash::from_slice(&hash),
        Ok(hash) if buf.is_empty() && hash.is_empty() => NodeHash::default(),
        _ => NodeHash::from_slice(rlp),
    }
}
