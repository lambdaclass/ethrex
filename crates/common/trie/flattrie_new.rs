use std::collections::HashMap;

use bytes::BufMut;
use ethereum_types::H256;
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::{
    constants::RLP_NULL,
    decode::{RLPDecode, decode_bytes},
    encode::{RLPEncode, encode_length},
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use rkyv::with::Skip;

use crate::{EMPTY_TRIE_HASH, Nibbles, Node as OldTrieNode, NodeHash, NodeRef, rlp::decode_child};

/// A trie implementation that is non recursive, POD and avoids deserialization
/// by providing views into a RLP flat buffer
#[derive(
    Default,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Serialize,
    rkyv::Deserialize,
    rkyv::Archive,
    Clone,
)]
pub struct FlatTrie {
    /// Contains the structural information of the MPT
    pub nodes: Vec<Node>,
    /// Stores a contiguous byte buffer with each initial RLP encoded node
    pub encoded_data: Vec<u8>,
    /// The index of the view for the root of this trie
    pub root_index: Option<usize>,
    /// Root hash that gets initialized when calling `Self::authenticate`
    #[serde(skip)]
    #[rkyv(with = Skip)]
    root_hash: Option<NodeHash>,
}

/// A view into a particular node
#[derive(
    Clone, serde::Serialize, serde::Deserialize, rkyv::Serialize, rkyv::Deserialize, rkyv::Archive,
)]
pub struct Node {
    pub handle: NodeHandle,
    pub encoded_range: Option<(usize, usize)>,
}

#[derive(
    Clone, serde::Serialize, serde::Deserialize, rkyv::Serialize, rkyv::Deserialize, rkyv::Archive,
)]
/// Contains information about this node type and who its children are.
/// Also contains overrides to the node's data.
///
/// The idea is that the initial data of the trie will be already encoded in RLP in a
/// contiguous buffer. Then insertions and removals will yield overrides over the encoded
/// data.
///
/// Finally the RLP buffer will be updated with the newest data based on the initial and overrides.
pub enum NodeHandle {
    Leaf {
        override_partial: Option<Nibbles>, // None means no override
        override_value: Option<Vec<u8>>,   // None means no override
    },
    Extension {
        override_prefix: Option<Nibbles>, // Nonde means no override
        child_index: Option<usize>,       // None means the child is pruned
    },
    Branch {
        children_indices: [Option<usize>; 16], // None means the child is pruned
    },
}

impl Node {}

impl FlatTrie {
    pub fn get(&self, path: &[u8]) -> Result<Option<&[u8]>, RLPDecodeError> {
        let mut path = Nibbles::from_bytes(path);
        fn recursive<'a>(
            trie: &'a FlatTrie,
            path: &mut Nibbles,
            index: usize,
        ) -> Result<Option<&'a [u8]>, RLPDecodeError> {
            let node = &trie.nodes[index];
            match node.handle {
                NodeHandle::Leaf {..} => {
                    let (partial, value) = trie.get_leaf_data(index)?;
                    if partial == *path {
                        return Ok(Some(value));
                    } else {
                        return Ok(None);
                    }
                }
                NodeHandle::Extension { child_index, .. } => {
                    let prefix = trie.get_extension_data(index)?;
                    if path.skip_prefix(prefix) {
                        recursive(trie, path, child_index.expect("no child for extension in get"))
                    } else {
                        Ok(None)
                    }
                }
                NodeHandle::Branch { children_indices, .. } => {
                    let Some(choice) = path.next_choice() else {
                        return Ok(None);
                    };
                    let Some(child_index) = children_indices[choice] else {
                        return Ok(None);
                    };
                    recursive(trie, path, child_index)
                }
            }
        }

        let Some(root_index) = self.root_index else {
            return Ok(None);
        };
        recursive(&self, &mut path, root_index)
    }

    /// Assumes this node index corresponds to a leaf, and retrieves its data taking into
    /// account the overrides.
    pub fn get_leaf_data(&self, index: usize) -> Result<(Nibbles, &[u8]), RLPDecodeError> {
        let handle = &self.nodes[index].handle;
        let NodeHandle::Leaf {
            override_partial,
            override_value,
        } = handle
        else {
            panic!("not leaf in get_leaf_data");
        };

        // TODO: put inside match
        let encoded_items = self.get_encoded_items(index)?;
        let data = match (override_partial, override_value) {
            (None, None) => {
                let (partial, _) = decode_bytes(encoded_items[0])?;
                let partial = Nibbles::decode_compact(partial);
                debug_assert!(partial.is_leaf());
                let (value, _) = decode_bytes(encoded_items[1])?;
                (partial, value)
            }
            (Some(partial), None) => {
                let (value, _) = decode_bytes(encoded_items[1])?;
                (partial.clone(), value)
            }
            (None, Some(value)) => {
                let (partial, _) = decode_bytes(encoded_items[0])?;
                let partial = Nibbles::decode_compact(partial);
                debug_assert!(partial.is_leaf());
                (partial, value.as_slice())
            }
            (Some(partial), Some(value)) => (partial.clone(), value.as_slice()),
        };
        Ok(data)
    }

    /// Assumes this node index corresponds to an extension, and retrieves its data taking into
    /// account the overrides.
    pub fn get_extension_data(&self, index: usize) -> Result<Nibbles, RLPDecodeError> {
        let handle = &self.nodes[index].handle;
        let NodeHandle::Extension {
            override_prefix, ..
        } = handle
        else {
            panic!("not leaf in get_leaf_data");
        };

        // TODO: put inside match
        let encoded_items = self.get_encoded_items(index)?;
        let data = match override_prefix {
            None => {
                let (prefix, _) = decode_bytes(encoded_items[0])?;
                let prefix = Nibbles::decode_compact(prefix);
                debug_assert!(!prefix.is_leaf());
                prefix
            }
            Some(prefix) => prefix.clone(),
        };
        Ok(data)
    }

    pub fn get_encoded_items(&self, index: usize) -> Result<Vec<&[u8]>, RLPDecodeError> {
        let node = &self.nodes[index];
        let encoded_range = node.encoded_range.expect("could not get encoded range");
        let data = &self.encoded_data[encoded_range.0..encoded_range.1];

        let mut decoder = Decoder::new(data)?;
        let mut rlp_items = Vec::with_capacity(17);
        while !decoder.is_done() && rlp_items.len() < 17 {
            let (item, new_decoder) = decoder.get_encoded_item_ref()?;
            decoder = new_decoder;
            rlp_items.push(item);
        }
        Ok(rlp_items)
    }
}
