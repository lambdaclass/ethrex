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

/// Contains information about this node type and who its children are.
/// Also contains overrides to the node's data.
///
/// The idea is that the initial data of the trie will be already encoded in RLP in a
/// contiguous buffer. Then insertions and removals will yield overrides over the encoded
/// data.
///
/// Finally the RLP buffer will be updated with the newest data based on the initial and overrides.
#[derive(
    Clone, serde::Serialize, serde::Deserialize, rkyv::Serialize, rkyv::Deserialize, rkyv::Archive,
)]
pub enum NodeHandle {
    Leaf {
        /// Overrides the encoded partial
        partial: Option<Nibbles>,
        /// Overrides the encoded value
        value: Option<Vec<u8>>,
    },
    Extension {
        /// Overrides the encoded prefix
        prefix: Option<Nibbles>,
        /// Reference to the child. If None, then the child is pruned.
        child_index: Option<usize>,
    },
    Branch {
        /// Reference to the children. If None, then the child is pruned.
        children_indices: [Option<usize>; 16],
    },
}

impl Node {}

impl FlatTrie {
    /// Get an element from the trie
    pub fn get(&self, path: &[u8]) -> Result<Option<&[u8]>, RLPDecodeError> {
        let mut path = Nibbles::from_bytes(path);
        fn recursive<'a>(
            trie: &'a FlatTrie,
            path: &mut Nibbles,
            index: usize,
        ) -> Result<Option<&'a [u8]>, RLPDecodeError> {
            let node = &trie.nodes[index];
            match node.handle {
                NodeHandle::Leaf { .. } => {
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
                        recursive(
                            trie,
                            path,
                            child_index.expect("no child for extension in get"),
                        )
                    } else {
                        Ok(None)
                    }
                }
                NodeHandle::Branch {
                    children_indices, ..
                } => {
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
            partial: override_partial,
            value: override_value,
        } = handle
        else {
            panic!("not leaf in get_leaf_data");
        };

        let data = match (override_partial, override_value) {
            (None, None) => {
                let encoded_items = self.get_encoded_items(index)?;
                let (partial, _) = decode_bytes(encoded_items[0])?;
                let partial = Nibbles::decode_compact(partial);
                debug_assert!(partial.is_leaf());
                let (value, _) = decode_bytes(encoded_items[1])?;
                (partial, value)
            }
            (Some(partial), None) => {
                let encoded_items = self.get_encoded_items(index)?;
                let (value, _) = decode_bytes(encoded_items[1])?;
                (partial.clone(), value)
            }
            (None, Some(value)) => {
                let encoded_items = self.get_encoded_items(index)?;
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
            prefix: override_prefix,
            ..
        } = handle
        else {
            panic!("not leaf in get_leaf_data");
        };

        let data = match override_prefix {
            None => {
                let encoded_items = self.get_encoded_items(index)?;
                let (prefix, _) = decode_bytes(encoded_items[0])?;
                let prefix = Nibbles::decode_compact(prefix);
                debug_assert!(!prefix.is_leaf());
                prefix
            }
            Some(prefix) => prefix.clone(),
        };
        Ok(data)
    }

    /// Gets the encoded items of a node based on its index.
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

    /// Overrides a node in the trie. Used whenever mutating the trie.
    ///
    /// An override can be used in the case of:
    /// 1. The data of some node gets updated
    /// 2. The children references of some node gets updated
    /// 3. A node is replaced with another
    pub fn override_node(&mut self, index: usize, override_node_handle: NodeHandle) -> usize {
        let original_node = self.nodes.get_mut(index).unwrap();

        let override_is_same_node_kind = matches!(
            (&original_node.handle, &override_node_handle),
            (NodeHandle::Leaf { .. }, NodeHandle::Leaf { .. })
                | (NodeHandle::Extension { .. }, NodeHandle::Extension { .. })
                | (NodeHandle::Branch { .. }, NodeHandle::Branch { .. })
        );

        // if node is not the same kind as the override, it gets replaced.
        if override_is_same_node_kind {
            let node = Node {
                handle: override_node_handle,
                encoded_range: None,
            };
            self.nodes.push(node);
            return self.nodes.len() - 1;
        }

        // else, mutate the handle
        match (&mut original_node.handle, override_node_handle) {
            (
                NodeHandle::Leaf {
                    partial: original_partial,
                    value: original_value,
                },
                NodeHandle::Leaf {
                    partial: override_partial,
                    value: override_value,
                },
            ) => {
                if let Some(override_partial) = override_partial {
                    *original_partial = Some(override_partial);
                }
                if let Some(override_value) = override_value {
                    *original_value = Some(override_value);
                }
            }
            (
                NodeHandle::Extension {
                    prefix: original_prefix,
                    child_index: original_child_index,
                },
                NodeHandle::Extension {
                    prefix: override_prefix,
                    child_index: override_child_index,
                },
            ) => {
                if let Some(override_prefix) = override_prefix {
                    *original_prefix = Some(override_prefix);
                }
                if let Some(override_child_index) = override_child_index {
                    *original_child_index = Some(override_child_index);
                }
            }
            (
                NodeHandle::Branch {
                    children_indices: original_children_indices,
                },
                NodeHandle::Branch {
                    children_indices: override_children_indices,
                },
            ) => {
                for i in 0..16 {
                    if override_children_indices[i].is_some() {
                        original_children_indices[i] = override_children_indices[i];
                    }
                }
            }
            _ => unreachable!(),
        }

        index
    }
}
