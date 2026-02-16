use std::array;

// Contains RLP encoding and decoding implementations for Trie Nodes
// This encoding is only used to store the nodes in the DB, it is not the encoding used for hash computation
use ethrex_rlp::{
    constants::RLP_NULL,
    decode::{RLPDecode, decode_bytes, decode_rlp_item, get_item_with_prefix},
    encode::{RLPEncode, encode_length},
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

use super::node::{BranchNode, ExtensionNode, LeafNode, Node};
use crate::{Nibbles, NodeHash, ValueRLP, db::TrieDB, error::TrieError};

impl RLPEncode for BranchNode {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let value_len = <[u8] as RLPEncode>::length(&self.value);
        let payload_len = self.choices.iter().fold(value_len, |acc, child| {
            acc + RLPEncode::length(child.compute_hash_ref())
        });

        encode_length(payload_len, buf);
        for child in self.choices.iter() {
            match child.compute_hash_ref() {
                NodeHash::Hashed(hash) => hash.0.encode(buf),
                NodeHash::Inline((_, 0)) => buf.put_u8(RLP_NULL),
                NodeHash::Inline((encoded, len)) => buf.put_slice(&encoded[..*len as usize]),
            }
        }
        <[u8] as RLPEncode>::encode(&self.value, buf);
    }

    // Duplicated to prealloc the buffer and avoid calculating the payload length twice
    fn encode_to_vec(&self) -> Vec<u8> {
        let value_len = <[u8] as RLPEncode>::length(&self.value);
        let choices_len = self.choices.iter().fold(0, |acc, child| {
            acc + RLPEncode::length(child.compute_hash_ref())
        });
        let payload_len = choices_len + value_len;

        let mut buf: Vec<u8> = Vec::with_capacity(payload_len + 3); // 3 byte prefix headroom

        encode_length(payload_len, &mut buf);
        for child in self.choices.iter() {
            match child.compute_hash_ref() {
                NodeHash::Hashed(hash) => hash.0.encode(&mut buf),
                NodeHash::Inline((_, 0)) => buf.push(RLP_NULL),
                NodeHash::Inline((encoded, len)) => {
                    buf.extend_from_slice(&encoded[..*len as usize])
                }
            }
        }
        <[u8] as RLPEncode>::encode(&self.value, &mut buf);

        buf
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
        let mut rlp_items_len = 0;
        let mut rlp_items: [Option<&[u8]>; 17] = Default::default();
        let mut decoder = Decoder::new(rlp)?;
        let mut item;
        // Get encoded fields

        // Check if we reached the end or if we decoded more items than the ones we need
        while !decoder.is_done() && rlp_items_len < 17 {
            (item, decoder) = decoder.get_encoded_item_ref()?;
            rlp_items[rlp_items_len] = Some(item);
            rlp_items_len += 1;
        }
        if !decoder.is_done() {
            return Err(RLPDecodeError::Custom(
                "Invalid arg count for Node, expected 2 or 17, got more than 17".to_string(),
            ));
        }
        // Deserialize into node depending on the available fields
        let node = match rlp_items_len {
            // Leaf or Extension Node
            2 => {
                let (path, _) = decode_bytes(rlp_items[0].expect("we already checked the length"))?;
                let path = Nibbles::decode_compact(path);
                if path.is_leaf() {
                    // Decode as Leaf
                    let (value, _) =
                        decode_bytes(rlp_items[1].expect("we already checked the length"))?;
                    LeafNode {
                        partial: path,
                        value: value.to_vec(),
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
                let (value, _) =
                    decode_bytes(rlp_items[16].expect("we already checked the length"))?;
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
        };
        Ok((node, decoder.finish()?))
    }
}

/// Perform a trie get directly on raw RLP bytes, avoiding full Node construction.
/// At each branch node, only the needed child is parsed (1 of 16) instead of all 17 items.
///
/// Bypasses Decoder entirely, working directly on raw byte slices with specialized
/// branch child skipping that handles the two common patterns (null: 0x80, hash: 0xa0+32)
/// via direct byte checks instead of general-purpose RLP parsing.
pub(crate) fn get_from_raw_rlp(
    db: &dyn TrieDB,
    initial_rlp: &[u8],
    mut path: Nibbles,
) -> Result<Option<ValueRLP>, TrieError> {
    let mut rlp_buf: Vec<u8> = initial_rlp.to_vec();

    loop {
        // Parse in a block so all borrows of rlp_buf are released before fetch_next_rlp.
        let child_hash = {
            // Extract list payload directly (no Decoder struct overhead)
            let (is_list, payload, _) =
                decode_rlp_item(&rlp_buf).map_err(TrieError::RLPDecode)?;
            if !is_list {
                return Err(TrieError::RLPDecode(RLPDecodeError::UnexpectedString));
            }

            // Parse first two items to detect node type
            let (item0, rest) =
                get_item_with_prefix(payload).map_err(TrieError::RLPDecode)?;
            let (item1, rest) =
                get_item_with_prefix(rest).map_err(TrieError::RLPDecode)?;

            if rest.is_empty() {
                // 2-item node: Leaf or Extension
                let (prefix_bytes, _) =
                    decode_bytes(item0).map_err(TrieError::RLPDecode)?;
                let prefix = Nibbles::decode_compact(prefix_bytes);

                if prefix.is_leaf() {
                    if path.skip_prefix(&prefix) {
                        let (value, _) =
                            decode_bytes(item1).map_err(TrieError::RLPDecode)?;
                        return Ok(Some(value.to_vec()));
                    }
                    return Ok(None);
                }
                // Extension
                if !path.skip_prefix(&prefix) {
                    return Ok(None);
                }
                decode_child(item1)
            } else {
                // 17-item node: Branch (items 0 and 1 already parsed, rest has items 2..16+value)
                if let Some(choice) = path.next_choice() {
                    if choice == 0 {
                        decode_child(item0)
                    } else if choice == 1 {
                        decode_child(item1)
                    } else {
                        // Skip from item 2 to item `choice` using fast branch child skipping
                        let remaining =
                            skip_branch_children(rest, choice - 2)?;
                        let (child_bytes, _) = get_item_with_prefix(remaining)
                            .map_err(TrieError::RLPDecode)?;
                        decode_child(child_bytes)
                    }
                } else {
                    // Path exhausted — return branch value (item 16)
                    // Skip items 2..15 (14 children) to reach item 16
                    let remaining = skip_branch_children(rest, 14)?;
                    let (value_bytes, _) = get_item_with_prefix(remaining)
                        .map_err(TrieError::RLPDecode)?;
                    let (value, _) =
                        decode_bytes(value_bytes).map_err(TrieError::RLPDecode)?;
                    return Ok(if value.is_empty() {
                        None
                    } else {
                        Some(value.to_vec())
                    });
                }
            }
        };
        if !child_hash.is_valid() {
            return Ok(None);
        }
        fetch_next_rlp(db, &child_hash, &path, &mut rlp_buf)?;
    }
}

/// Skip `n` branch children in raw RLP bytes using fast-path pattern matching.
///
/// Branch children are almost always one of two patterns:
/// - `0x80`: null/empty child (1 byte) — the RLP encoding of an empty byte string
/// - `0xa0` + 32 bytes: a 32-byte hash (33 bytes total) — the RLP encoding of a hash
///
/// These two patterns are handled with direct byte checks, avoiding the overhead of
/// general-purpose `decode_rlp_item` + `get_item_with_prefix`. Rare cases (inline nodes,
/// single-byte values) fall back to `get_item_with_prefix`.
#[inline]
fn skip_branch_children(mut data: &[u8], n: usize) -> Result<&[u8], TrieError> {
    for _ in 0..n {
        if data.is_empty() {
            return Err(TrieError::RLPDecode(RLPDecodeError::InvalidLength));
        }
        data = match data[0] {
            // Null child: 0x80 = RLP empty string (1 byte)
            0x80 => &data[1..],
            // Hash child: 0xa0 = RLP 32-byte string prefix (1 + 32 = 33 bytes)
            0xa0 => {
                if data.len() < 33 {
                    return Err(TrieError::RLPDecode(RLPDecodeError::InvalidLength));
                }
                &data[33..]
            }
            // Rare: single byte value [0x00..0x7f] or inline node — use general parser
            _ => {
                let (_, rest) =
                    get_item_with_prefix(data).map_err(TrieError::RLPDecode)?;
                rest
            }
        };
    }
    Ok(data)
}

/// Fetch the next node's RLP into the buffer, reusing its allocation.
fn fetch_next_rlp(
    db: &dyn TrieDB,
    child_hash: &NodeHash,
    path: &Nibbles,
    buf: &mut Vec<u8>,
) -> Result<(), TrieError> {
    buf.clear();
    match child_hash {
        NodeHash::Inline(_) => {
            buf.extend_from_slice(child_hash.as_ref());
        }
        NodeHash::Hashed(_) => {
            let data = db
                .get(path.current())?
                .filter(|rlp| !rlp.is_empty())
                .ok_or_else(|| {
                    TrieError::Verify("Child node not found in database".to_string())
                })?;
            buf.extend_from_slice(&data);
        }
    }
    Ok(())
}

fn decode_child(rlp: &[u8]) -> NodeHash {
    match decode_bytes(rlp) {
        Ok((hash, &[])) if hash.len() == 32 => NodeHash::from_slice(hash),
        Ok((&[], &[])) => NodeHash::default(),
        _ => NodeHash::from_slice(rlp),
    }
}
