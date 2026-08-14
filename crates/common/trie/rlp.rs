#[cfg(not(feature = "std"))]
use alloc::{string::ToString, vec::Vec};
use core::array;

// Contains RLP encoding and decoding implementations for Trie Nodes
// This encoding is only used to store the nodes in the DB, it is not the encoding used for hash computation
use ethrex_rlp::{
    constants::RLP_NULL,
    decode::{RLPDecode, decode_bytes},
    encode::{RLPEncode, encode_length},
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

use ethrex_crypto::NativeCrypto;

use super::node::{BranchNode, ExtensionNode, LeafNode, Node};
use crate::{Nibbles, NodeHash};

// SAFETY: `NativeCrypto` is used here instead of a `&dyn Crypto` parameter because
// `RLPEncode` is a fixed trait signature that cannot accept extra parameters.
// This is safe in the `commit()` path: `NodeRef::commit()` recursively populates
// child `OnceLock` hashes before calling `encode()`, so `compute_hash_ref` returns
// cached values without invoking keccak. If `encode()` were called on uncommitted
// nodes (e.g. from `put_batch_no_alloc`), `NativeCrypto` would be used and the
// result stored in the `OnceLock` — but this only happens in native storage paths
// where `NativeCrypto` is the correct provider.
impl RLPEncode for BranchNode {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        // Resolve each child's hash once: the length pass and the encode pass
        // both needed it, so a 16-choice branch was paying 32 resolutions.
        let hashes: [&NodeHash; 16] =
            array::from_fn(|i| self.choices[i].compute_hash_ref(&NativeCrypto));

        let value_len = <[u8] as RLPEncode>::length(&self.value);
        let payload_len = hashes
            .iter()
            .fold(value_len, |acc, hash| acc + RLPEncode::length(*hash));

        encode_length(payload_len, buf);
        for hash in hashes {
            match hash {
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
            acc + RLPEncode::length(child.compute_hash_ref(&NativeCrypto))
        });
        let payload_len = choices_len + value_len;

        let mut buf: Vec<u8> = Vec::with_capacity(payload_len + 3); // 3 byte prefix headroom

        encode_length(payload_len, &mut buf);
        for child in self.choices.iter() {
            match child.compute_hash_ref(&NativeCrypto) {
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

impl BranchNode {
    /// Concrete-typed sibling of `<Self as RLPEncode>::encode`, appending into a
    /// `Vec<u8>` rather than a `&mut dyn BufMut`.
    ///
    /// `RLPEncode::encode` must take a trait object, so hashing a branch paid a
    /// vtable dispatch for each of its ~41 `put_u8`/`put_slice` calls. Hashing is
    /// the hot consumer (every `memoize_hashes` walk re-encodes each dirty
    /// branch), so it gets a monomorphic path; `RLPEncode::encode` is untouched
    /// for every other caller.
    pub fn encode_into_vec(&self, buf: &mut Vec<u8>) {
        let hashes: [&NodeHash; 16] =
            array::from_fn(|i| self.choices[i].compute_hash_ref(&NativeCrypto));

        let value_len = <[u8] as RLPEncode>::length(&self.value);
        let payload_len = hashes
            .iter()
            .fold(value_len, |acc, hash| acc + RLPEncode::length(*hash));

        encode_length(payload_len, buf);
        for hash in hashes {
            match hash {
                // Written directly rather than through `RLPEncode::encode`, which
                // would take a trait object and pay a virtual call per byte group.
                NodeHash::Hashed(hash) => {
                    buf.push(RLP_NULL + 32);
                    buf.extend_from_slice(&hash.0);
                }
                NodeHash::Inline((_, 0)) => buf.push(RLP_NULL),
                NodeHash::Inline((encoded, len)) => {
                    buf.extend_from_slice(&encoded[..*len as usize])
                }
            }
        }
        put_rlp_bytes(&self.value, buf);
    }
}

impl RLPEncode for ExtensionNode {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let mut encoder = Encoder::new(buf).encode_bytes(&self.prefix.encode_compact());
        encoder = self.child.compute_hash(&NativeCrypto).encode(encoder);
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

// ── Allocation-free node encoding for the hashing path ───────────────────────
//
// `RLPEncode::encode` has to take a `&mut dyn BufMut`, and `Encoder` accumulates
// a node's payload in a fresh heap `Vec` so it can prepend the list header once
// the payload length is known — then copies the whole payload across. Branch
// nodes already sidestep both (see `BranchNode::encode_into_vec` above); leaf and
// extension did not, so hashing one cost an allocation for the hex-prefix path,
// several more growing the encoder's scratch buffer, and a full second pass over
// the bytes.
//
// Every node's payload length is computable before any of it is written, so the
// header can go down first and the node can be built in place. That matters most
// in the zkVM guest, whose bump allocator never reclaims and whose `realloc`
// always allocates fresh and memcpys, so allocation churn is permanently
// consumed heap rather than reused blocks.
//
// `RLPEncode::encode` is left untouched for every other caller.

/// RLP-encodes a byte string into a concrete `Vec`. Byte-identical to
/// `<[u8] as RLPEncode>::encode`, without the trait object.
#[inline]
fn put_rlp_bytes(value: &[u8], buf: &mut Vec<u8>) {
    if let [single] = value
        && *single < RLP_NULL
    {
        buf.push(*single);
    } else if value.len() < 56 {
        buf.push(RLP_NULL + value.len() as u8);
        buf.extend_from_slice(value);
    } else {
        let be = value.len().to_be_bytes();
        let start = be.iter().position(|&x| x != 0).unwrap_or(be.len() - 1);
        buf.push(0xb7 + (be.len() - start) as u8);
        buf.extend_from_slice(&be[start..]);
        buf.extend_from_slice(value);
    }
}

/// Byte length of the RLP encoding of a hex-prefix path of `compact_len` bytes.
///
/// The compact form's first byte is the hex-prefix header — at most `0x3f` — so a
/// one-byte path is always its own RLP encoding, and that can be decided from the
/// length alone. A trie path is at most 64 nibbles, so `compact_len` never
/// exceeds 33 and the long-form arm is unreachable; it is spelled out anyway
/// rather than assumed.
#[inline]
const fn compact_path_rlp_len(compact_len: usize) -> usize {
    if compact_len == 1 {
        1
    } else if compact_len < 56 {
        1 + compact_len
    } else {
        1 + (compact_len.ilog2() as usize / 8 + 1) + compact_len
    }
}

/// Writes the RLP header for a hex-prefix path of `compact_len` bytes; the
/// payload follows from `Nibbles::encode_compact_into`. Pairs with
/// [`compact_path_rlp_len`] — the two must agree byte for byte.
#[inline]
fn put_compact_path_header(compact_len: usize, buf: &mut Vec<u8>) {
    if compact_len == 1 {
        // Encoded as the bare byte, which `encode_compact_into` writes itself.
    } else if compact_len < 56 {
        buf.push(RLP_NULL + compact_len as u8);
    } else {
        let be = compact_len.to_be_bytes();
        let start = be.iter().position(|&x| x != 0).unwrap_or(be.len() - 1);
        buf.push(0xb7 + (be.len() - start) as u8);
        buf.extend_from_slice(&be[start..]);
    }
}

/// Bytes an extension node's child reference occupies, mirroring
/// `NodeHash::encode`: a 32-byte hash becomes an RLP byte string, an inline node
/// is spliced in verbatim. An empty inline hash contributes nothing, which is
/// what the `Encoder` path produces today — it only arises from a malformed
/// trie, and this is not the place to start rejecting one.
#[inline]
const fn extension_child_len(hash: &NodeHash) -> usize {
    match hash {
        NodeHash::Hashed(_) => 33,
        NodeHash::Inline((_, len)) => *len as usize,
    }
}

/// Writes an extension node's child reference. Pairs with
/// [`extension_child_len`].
#[inline]
fn put_extension_child(hash: &NodeHash, buf: &mut Vec<u8>) {
    match hash {
        NodeHash::Hashed(hash) => {
            buf.push(RLP_NULL + 32);
            buf.extend_from_slice(&hash.0);
        }
        NodeHash::Inline((encoded, len)) => buf.extend_from_slice(&encoded[..*len as usize]),
    }
}

impl LeafNode {
    /// Concrete-typed sibling of `<Self as RLPEncode>::encode`, building the node
    /// directly into `buf`. See the module note above.
    pub fn encode_into_vec(&self, buf: &mut Vec<u8>) {
        let path_len = self.partial.encode_compact_len();
        let payload_len =
            compact_path_rlp_len(path_len) + <[u8] as RLPEncode>::length(&self.value);

        encode_length(payload_len, buf);
        put_compact_path_header(path_len, buf);
        self.partial.encode_compact_into(buf);
        put_rlp_bytes(&self.value, buf);
    }
}

impl ExtensionNode {
    /// Concrete-typed sibling of `<Self as RLPEncode>::encode`, building the node
    /// directly into `buf`. See the module note above.
    ///
    /// Reads the child's hash through `compute_hash_ref`, which returns the
    /// memoized value on the hashing path — `memoize_hashes` populates children
    /// before their parent is encoded, so `NativeCrypto` is never invoked here.
    pub fn encode_into_vec(&self, buf: &mut Vec<u8>) {
        let path_len = self.prefix.encode_compact_len();
        let child = self.child.compute_hash_ref(&NativeCrypto);
        let payload_len = compact_path_rlp_len(path_len) + extension_child_len(child);

        encode_length(payload_len, buf);
        put_compact_path_header(path_len, buf);
        self.prefix.encode_compact_into(buf);
        put_extension_child(child, buf);
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

fn decode_child(rlp: &[u8]) -> NodeHash {
    match decode_bytes(rlp) {
        Ok((hash, &[])) if hash.len() == 32 => NodeHash::from_slice(hash),
        Ok((&[], &[])) => NodeHash::default(),
        _ => NodeHash::from_slice(rlp),
    }
}
