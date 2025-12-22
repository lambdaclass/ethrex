use bytes::BufMut;
use ethrex_rlp::{
    constants::RLP_NULL,
    decode::{RLPDecode, decode_bytes},
    encode::{RLPEncode, encode_length},
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use rkyv::with::Skip;
use thiserror::Error;

use crate::{
    EMPTY_TRIE_HASH, Nibbles, Node as EthrexTrieNode, NodeHash, NodeRef as EthrexTrieNodeRef,
    rlp::decode_child,
};

/// A trie implementation optimal for zkVM environments.
///
/// What makes this optimal is that:
/// 1. All nodes of the initial state of the trie are stored RLP-encoded in a contiguous buffer of bytes,
///    and are referred to by ranges. This avoids deserialization.
/// 2. Nodes are indexed by an integer, which avoids recursive structures and references.
/// 3. Structural information is stored separately (the `NodeType` struct).
/// 4. New node data (like new nodes or node mutations) are stored directly in memory, outside the data buffer,
///    overriding the encoded data. The trie only encodes when necessary (hashing).
/// 5. Distinguishes hashing from authentication (the latter is used to check initial state, the former to check for final state).
#[derive(
    Default,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Serialize,
    rkyv::Deserialize,
    rkyv::Archive,
    Clone,
)]
pub struct EncodedTrie {
    /// Contains the structural information of the MPT
    pub nodes: Vec<Node>,
    /// Stores a contiguous byte buffer with each initial RLP encoded node
    pub encoded_data: Vec<u8>,
    /// The index of the root node
    pub root_index: Option<usize>,
    /// Node hashes get cached when authenticating or hashing the trie for the first time
    #[serde(skip)]
    #[rkyv(with = Skip)]
    hashes: Vec<Option<NodeHash>>,
}

/// A handle into a particular node
#[derive(
    Clone, serde::Serialize, serde::Deserialize, rkyv::Serialize, rkyv::Deserialize, rkyv::Archive,
)]
pub struct Node {
    pub node_type: NodeType,
    pub encoded_range: Option<(usize, usize)>,
}

/// Contains information about this node type and references to its children.
/// Also contains data overrides.
///
/// A trie stores all its initial nodes RLP-encoded in a contiguous byte buffer, but on each
/// insert or remove nodes are mutated. These are represented by overriding the encoded data.
/// New nodes don't have encoded data, so their data is stored just as overrides.
///
/// This results in that we only have to encode the final state of the trie, when we are interested
/// in calculating the final root hash.
#[derive(
    Clone, serde::Serialize, serde::Deserialize, rkyv::Serialize, rkyv::Deserialize, rkyv::Archive,
)]
pub enum NodeType {
    Leaf {
        /// Overrides the partial
        partial: Option<Nibbles>,
        /// Overrides the value
        value: Option<Vec<u8>>,
    },
    Extension {
        /// Overrides the prefix
        prefix: Option<Nibbles>,
        /// Reference to the child. If None, then the child is pruned.
        child_index: Option<usize>,
    },
    Branch {
        /// Reference to the children.
        /// - If None, then there is no child for that choice.
        /// - If Some(None), then there is a child but its pruned.
        children_indices: [Option<Option<usize>>; 16],
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EncodedTrieError {
    #[error("Node of index {0} not found")]
    NodeNotFound(usize),
    #[error("Node hash of index {0} not found")]
    NodeHashNotFound(usize),
    #[error("Node of index {0} doesn't have encoded data")]
    NonEncodedNode(usize),
    #[error("RLPDecodeError")]
    RLPDecodeError(#[from] RLPDecodeError),
}

impl EncodedTrie {
    /// Get an element from the trie
    pub fn get(&self, path: &[u8]) -> Result<Option<&[u8]>, RLPDecodeError> {
        let mut path = Nibbles::from_bytes(path);
        fn recursive<'a>(
            trie: &'a EncodedTrie,
            path: &mut Nibbles,
            index: usize,
        ) -> Result<Option<&'a [u8]>, RLPDecodeError> {
            let node = &trie.nodes[index];
            match node.node_type {
                NodeType::Leaf { .. } => {
                    let (partial, value) = trie.get_leaf_data(index)?;
                    if partial == *path {
                        Ok(Some(value))
                    } else {
                        Ok(None)
                    }
                }
                NodeType::Extension { child_index, .. } => {
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
                NodeType::Branch {
                    children_indices, ..
                } => {
                    let Some(choice) = path.next_choice() else {
                        return Ok(None);
                    };
                    let Some(child_index) = children_indices[choice] else {
                        return Ok(None);
                    };
                    recursive(trie, path, child_index.expect("pruned branch child"))
                }
            }
        }

        let Some(root_index) = self.root_index else {
            return Ok(None);
        };
        recursive(self, &mut path, root_index)
    }

    /// Inserts an element in the trie.
    ///
    /// This also clears the hashes of every relevant node.
    pub fn insert(&mut self, path: Vec<u8>, value: Vec<u8>) -> Result<(), EncodedTrieError> {
        let path = Nibbles::from_bytes(&path);
        if let Some(root_index) = self.root_index {
            self.root_index = self.insert_inner(root_index, path, value).map(Some)?;
        } else {
            self.root_index = Some(self.put_leaf(path, value));
        }
        Ok(())
    }

    fn insert_inner(
        &mut self,
        self_index: usize,
        mut path: Nibbles,
        value: Vec<u8>,
    ) -> Result<usize, EncodedTrieError> {
        self.hashes[self_index] = None;
        let self_view = &self.nodes[self_index];
        match self_view.node_type {
            NodeType::Leaf { .. } => {
                let (partial, _) = self.get_leaf_data(self_index)?;
                if partial == path {
                    let override_node_handle = NodeType::Leaf {
                        partial: None,
                        value: Some(value),
                    };
                    Ok(self.override_node(self_index, override_node_handle)?)
                } else {
                    // Current node will be replaced with a branch or extension node
                    let match_index = path.count_prefix(&partial);
                    let self_choice_idx = partial.at(match_index);
                    let new_leaf_choice_idx = path.at(match_index);

                    // Modify the partial of self
                    let new_self_index = self.override_node(
                        self_index,
                        NodeType::Leaf {
                            partial: Some(partial.offset(match_index + 1)),
                            value: None,
                        },
                    )?;

                    debug_assert!(
                        self_choice_idx != 16,
                        "leaf insertion yielded branch with old value"
                    );
                    debug_assert!(
                        new_leaf_choice_idx != 16,
                        "leaf insertion yielded branch with new value"
                    );
                    // Yields a new leaf with the path and value in it, and a new branch
                    // with the new and old leaf as children.
                    let new_leaf_index = self.put_leaf(path.offset(match_index + 1), value);
                    let branch_index = {
                        let mut children_indices = [None; 16];
                        children_indices[new_leaf_choice_idx] = Some(Some(new_leaf_index));
                        children_indices[self_choice_idx] = Some(Some(new_self_index));
                        self.put_node(NodeType::Branch { children_indices })
                    };

                    if match_index == 0 {
                        Ok(branch_index)
                    } else {
                        // Yields an extension node with the branch as child
                        Ok(self.put_node(NodeType::Extension {
                            prefix: Some(path.slice(0, match_index)),
                            child_index: Some(branch_index),
                        }))
                    }
                }
            }
            NodeType::Extension { child_index, .. } => {
                let prefix = self.get_extension_data(self_index)?;
                let match_index = path.count_prefix(&prefix);
                if match_index == prefix.len() {
                    let path = path.offset(match_index);
                    let new_child_index = self.insert_inner(
                        child_index
                            .expect("missing child of extension node at match_index == prefix"),
                        path,
                        value,
                    )?;
                    Ok(self.override_node(
                        self_index,
                        NodeType::Extension {
                            prefix: None,
                            child_index: Some(new_child_index),
                        },
                    )?)
                } else if match_index == 0 {
                    debug_assert!(
                        prefix.at(0) != 16,
                        "insertion into extension yielded branch with value"
                    );
                    let branch_index = if prefix.len() == 1 {
                        let mut children_indices = [None; 16];
                        children_indices[prefix.at(0)] = Some(child_index);
                        if child_index.is_some() {
                            self.put_node(NodeType::Branch { children_indices })
                        } else {
                            // pruned child, we must make its hash available by encoding the branch
                            // TODO: hacky
                            let child_hash = self.get_extension_encoded_child_hash(self_index)?;
                            let mut children_hashes = [None; 16];
                            children_hashes[prefix.at(0)] = Some(child_hash);
                            let encoded = encode_branch(children_hashes);
                            self.put_node_encoded(NodeType::Branch { children_indices }, encoded)
                        }
                    } else {
                        // New extension with self_node as a child
                        let handle = NodeType::Extension {
                            prefix: Some(prefix.offset(1)),
                            child_index,
                        };
                        let new_node_index = if child_index.is_some() {
                            self.put_node(handle)
                        } else {
                            // pruned child, we must make its hash available by encoding the branch
                            // TODO: hacky
                            let child_hash = self.get_extension_encoded_child_hash(self_index)?;
                            let encoded = encode_extension(prefix.offset(1), child_hash);
                            self.put_node_encoded(handle, encoded)
                        };
                        {
                            let mut children_indices = [None; 16];
                            children_indices[prefix.at(0)] = Some(Some(new_node_index));
                            self.put_node(NodeType::Branch { children_indices })
                        }
                    };
                    self.insert_inner(branch_index, path, value)
                } else {
                    let new_extension_index = self.override_node(
                        self_index,
                        NodeType::Extension {
                            prefix: Some(prefix.offset(match_index)),
                            child_index,
                        },
                    )?;
                    let new_node_index =
                        self.insert_inner(new_extension_index, path.offset(match_index), value)?;
                    Ok(self.put_node(NodeType::Extension {
                        prefix: Some(prefix.slice(0, match_index)),
                        child_index: Some(new_node_index),
                    }))
                }
            }
            NodeType::Branch {
                mut children_indices,
            } => {
                let choice = path
                    .next_choice()
                    .expect("branch insertion yielded value on a branch");
                let new_child_index = match children_indices[choice] {
                    Some(None) => {
                        panic!("Missing children of branch needed for insert")
                    }
                    None => self.put_leaf(path, value),
                    Some(Some(index)) => self.insert_inner(index, path, value)?,
                };
                children_indices[choice] = Some(Some(new_child_index));
                self.override_node(self_index, NodeType::Branch { children_indices })
            }
        }
    }

    /// Removes an element from the trie.
    ///
    /// This also clears the hashes of every relevant node.
    pub fn remove(&mut self, path: &[u8]) -> Result<(), EncodedTrieError> {
        let path = Nibbles::from_bytes(path);
        if let Some(root_index) = self.root_index {
            self.root_index = self.remove_inner(root_index, path)?;
        }
        Ok(())
    }

    pub fn remove_inner(
        &mut self,
        index: usize,
        mut path: Nibbles,
    ) -> Result<Option<usize>, EncodedTrieError> {
        self.hashes[index] = None;
        let node = &self.nodes[index];
        match node.node_type {
            NodeType::Leaf { .. } => {
                let (partial, _) = self.get_leaf_data(index)?;
                if partial == path {
                    Ok(None)
                } else {
                    Ok(Some(index))
                }
            }
            NodeType::Extension { child_index, .. } => {
                let mut prefix = self.get_extension_data(index)?;

                if !path.skip_prefix(&prefix) {
                    return Ok(Some(index));
                }
                let new_child_index = self.remove_inner(
                    child_index.expect("missing child of extension node at remove"),
                    path,
                )?;
                let Some(new_child_index) = new_child_index else {
                    return Ok(None);
                };

                let new_child = &self.nodes[new_child_index];
                let new_index = match new_child.node_type {
                    NodeType::Branch { .. } => {
                        let handle = NodeType::Extension {
                            prefix: Some(prefix),
                            child_index: Some(new_child_index),
                        };
                        self.put_node(handle)
                    }
                    NodeType::Extension { child_index, .. } => {
                        let new_child_prefix = self.get_extension_data(new_child_index)?;
                        prefix.extend(&new_child_prefix);
                        let handle = NodeType::Extension {
                            prefix: Some(prefix),
                            child_index,
                        };
                        self.override_node(index, handle)?
                    }
                    NodeType::Leaf { .. } => {
                        let (partial, value) = self.get_leaf_data(new_child_index)?;
                        prefix.extend(&partial);
                        let handle = NodeType::Leaf {
                            partial: Some(prefix),
                            value: Some(value.to_vec()),
                        };
                        self.put_node(handle)
                    }
                };
                Ok(Some(new_index))
            }
            NodeType::Branch {
                mut children_indices,
            } => {
                let choice = path
                    .next_choice()
                    .expect("branch removal yielded value on a branch");

                let Some(child_index) = children_indices[choice] else {
                    return Ok(Some(index));
                };

                let new_child_index = self.remove_inner(
                    child_index.expect("pruned branch choice needed for remove"),
                    path,
                )?;
                children_indices[choice] = new_child_index.map(Some);

                let new_valid_children: Vec<_> = children_indices
                    .iter()
                    .enumerate()
                    .filter_map(|(i, c)| c.map(|c| (i, c)))
                    .collect();

                match new_valid_children.len() {
                    0 => Ok(None),
                    1 => {
                        let (choice_idx, child_idx) = new_valid_children[0];
                        let child_idx = child_idx.expect("missing child of branch at remove");
                        let child = &self.nodes[child_idx];

                        match child.node_type {
                            NodeType::Leaf { .. } => {
                                let (mut partial, value) = self.get_leaf_data(child_idx)?;
                                partial.prepend(choice_idx as u8);
                                Ok(Some(self.put_leaf(partial, value.to_vec())))
                            }
                            NodeType::Extension { child_index, .. } => {
                                let mut prefix = self.get_extension_data(child_idx)?;
                                prefix.prepend(choice_idx as u8);
                                let child_index = child_index
                                    .expect("missing child of extension at remove for branch case");
                                let handle = NodeType::Extension {
                                    prefix: Some(prefix),
                                    child_index: Some(child_index),
                                };
                                Ok(Some(self.put_node(handle)))
                            }
                            NodeType::Branch { .. } => {
                                let prefix = Nibbles::from_hex(vec![choice_idx as u8]);
                                let handle = NodeType::Extension {
                                    prefix: Some(prefix),
                                    child_index: Some(child_idx),
                                };
                                Ok(Some(self.put_node(handle)))
                            }
                        }
                    }
                    _ => {
                        let handle = NodeType::Branch { children_indices };
                        self.override_node(index, handle).map(Some)
                    }
                }
            }
        }
    }

    /// Adds a new node to the trie with a specific handle
    ///
    /// # Warning
    /// The node's data will be stored in its overrides because there is no underlying
    /// encoded node.
    pub fn put_node(&mut self, node_type: NodeType) -> usize {
        let node = Node {
            node_type,
            encoded_range: None,
        };
        self.nodes.push(node);
        self.hashes.push(None);
        self.nodes.len() - 1
    }

    /// Adds a new node to the trie, already encoded.
    pub fn put_node_encoded(&mut self, node_type: NodeType, encoded: Vec<u8>) -> usize {
        let start = self.encoded_data.len();
        self.encoded_data.extend(encoded);
        let end = self.encoded_data.len();
        let node = Node {
            node_type,
            encoded_range: Some((start, end)),
        };
        self.nodes.push(node);
        self.hashes.push(None);
        self.nodes.len() - 1
    }

    /// Puts a new leaf node from a prefix and a value.
    pub fn put_leaf(&mut self, partial: Nibbles, value: Vec<u8>) -> usize {
        let handle = NodeType::Leaf {
            partial: Some(partial),
            value: Some(value),
        };
        self.put_node(handle)
    }

    /// Overrides a node in the trie. Used whenever mutating the trie.
    ///
    /// An override can be used in the case of:
    /// 1. The data of some node gets updated
    /// 2. The children references of some node gets updated
    /// 3. A node is replaced with another
    pub fn override_node(
        &mut self,
        index: usize,
        override_node_handle: NodeType,
    ) -> Result<usize, EncodedTrieError> {
        let Some(original_node) = self.nodes.get_mut(index) else {
            return Err(EncodedTrieError::NodeNotFound(index));
        };

        let override_is_same_node_kind = matches!(
            (&original_node.node_type, &override_node_handle),
            (NodeType::Leaf { .. }, NodeType::Leaf { .. })
                | (NodeType::Extension { .. }, NodeType::Extension { .. })
                | (NodeType::Branch { .. }, NodeType::Branch { .. })
        );

        // if node is not the same kind as the override, panic
        // we should use put_node() in these cases
        if !override_is_same_node_kind {
            panic!();
        }

        // else, mutate the handle
        match (&mut original_node.node_type, override_node_handle) {
            (
                NodeType::Leaf {
                    partial: original_partial,
                    value: original_value,
                },
                NodeType::Leaf {
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
                NodeType::Extension {
                    prefix: original_prefix,
                    child_index: original_child_index,
                },
                NodeType::Extension {
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
                NodeType::Branch {
                    children_indices: original_children_indices,
                },
                NodeType::Branch {
                    children_indices: override_children_indices,
                },
            ) => {
                *original_children_indices = override_children_indices;
            }
            _ => unreachable!(),
        }

        Ok(index)
    }

    /// Hashes all encoded nodes before any changes to the trie, checking consistency across
    /// encoded (non-pruned) nodes to make sure they reference valid children hashes.
    ///
    /// Returns a root hash that binds to the trie structure and data, effectively authenticating
    /// the trie.
    ///
    /// # Warning
    /// This also allocates the `Self::hashes` vector, clearing all cached hashes, so this
    /// should only be called once per trie.
    pub fn authenticate(&mut self) -> Result<NodeHash, EncodedTrieError> {
        self.hashes = vec![None; self.nodes.len()];
        fn recursive(trie: &mut EncodedTrie, index: usize) -> Result<(), EncodedTrieError> {
            if trie.hashes[index].is_some() {
                return Ok(());
            }
            match &trie.nodes[index].node_type.clone() {
                NodeType::Leaf { .. } => {}
                NodeType::Extension { child_index, .. } => {
                    if let Some(child_index) = child_index {
                        recursive(trie, *child_index)?;
                        let child_hash = trie.get_hash(*child_index)?;
                        let encoded_items = trie.get_encoded_items(index)?;
                        let encoded_child_hash = decode_bytes(encoded_items[1])?.0;
                        if child_hash.as_ref() != encoded_child_hash {
                            panic!("invalid encoded child hash for extension node");
                        }
                    }
                }
                NodeType::Branch { children_indices } => {
                    for (_, child_index) in children_indices
                        .iter()
                        .enumerate()
                        .filter_map(|(i, c)| c.flatten().map(|c| (i, c)))
                    {
                        recursive(trie, child_index)?;
                    }

                    let encoded_items = trie.get_encoded_items(index)?;
                    for (i, child_index) in children_indices
                        .iter()
                        .enumerate()
                        .filter_map(|(i, c)| c.flatten().map(|c| (i, c)))
                    {
                        let child_hash = trie.get_hash(child_index)?;
                        let encoded_child_hash = decode_bytes(encoded_items[i])?.0;
                        if child_hash.as_ref() != encoded_child_hash {
                            panic!("invalid encoded child hash for branch node");
                        }
                    }
                }
            }

            let hash = trie.hash_encoded_data(index)?;
            trie.hashes[index] = Some(hash);
            Ok(())
        }
        let Some(root_index) = self.root_index else {
            return Ok((*EMPTY_TRIE_HASH).into());
        };
        recursive(self, root_index)?;
        self.get_hash(root_index)
    }

    /// Hashes all encoded nodes after changes to the trie, applying overrides and re-encoding
    /// if necessary.
    ///
    /// Returns a root hash that binds to the trie structure and data.
    pub fn hash(&mut self) -> Result<NodeHash, EncodedTrieError> {
        fn recursive(trie: &mut EncodedTrie, index: usize) -> Result<(), EncodedTrieError> {
            if trie.hashes[index].is_some() {
                return Ok(());
            }
            match &trie.nodes[index].node_type.clone() {
                NodeType::Leaf { partial, value } => {
                    if partial.is_some() || value.is_some() {
                        // re-encode with new values
                        let (partial, value) = trie.get_leaf_data(index)?;
                        let encoded = encode_leaf(partial, value);
                        trie.hashes[index] = Some(NodeHash::from_encoded(&encoded));
                    } else {
                        // use already encoded
                        trie.hashes[index] = trie.hash_encoded_data(index).map(Some)?;
                    }
                }
                NodeType::Extension {
                    prefix,
                    child_index,
                } => match (prefix, child_index) {
                    (None, None) => {
                        trie.hashes[index] = trie.hash_encoded_data(index).map(Some)?;
                    }
                    (_, Some(child_index)) => {
                        // recurse to calculate the child hash and re-encode
                        recursive(trie, *child_index)?;
                        let child_hash = trie.get_hash(*child_index)?;
                        let prefix = trie.get_extension_data(index)?;
                        let encoded = encode_extension(prefix, child_hash);
                        trie.hashes[index] = Some(NodeHash::from_encoded(&encoded));
                    }
                    (Some(prefix), None) => {
                        // get encoded child hash and re-encode
                        let child_hash = trie.get_extension_encoded_child_hash(index)?;
                        let encoded = encode_extension(prefix.clone(), child_hash);
                        trie.hashes[index] = Some(NodeHash::from_encoded(&encoded));
                    }
                },
                NodeType::Branch { children_indices } => {
                    let mut any_pruned = false;
                    for child_index in children_indices.iter().flatten() {
                        if let Some(child_index) = child_index {
                            recursive(trie, *child_index)?;
                        } else {
                            any_pruned = true;
                        }
                    }

                    let mut children_hashes: [Option<NodeHash>; 16] = [None; 16];

                    if any_pruned {
                        let encoded_items = trie.get_encoded_items(index)?;
                        for (i, child) in children_indices.iter().enumerate() {
                            let Some(child_index) = child else {
                                // no child for this index
                                continue;
                            };

                            if let Some(child_index) = child_index {
                                children_hashes[i] = trie.get_hash(*child_index).map(Some)?;
                            } else {
                                children_hashes[i] = Some(decode_child(encoded_items[i]))
                            }
                        }
                    } else {
                        for (i, child) in children_indices.iter().enumerate() {
                            let Some(child_index) = child else {
                                // no child for this index
                                continue;
                            };

                            if let Some(child_index) = child_index {
                                children_hashes[i] = trie.get_hash(*child_index).map(Some)?;
                            }
                        }
                    }

                    let encoded = encode_branch(children_hashes);
                    trie.hashes[index] = Some(NodeHash::from_encoded(&encoded));
                }
            }
            Ok(())
        }
        let Some(root_index) = self.root_index else {
            return Ok((*EMPTY_TRIE_HASH).into());
        };
        recursive(self, root_index)?;
        self.get_hash(root_index)
    }

    /// Hashes the encoded data of some node index.
    ///
    /// # Warning
    /// The encoded data might be outdated. This function does not take into account overrides.
    pub fn hash_encoded_data(&self, index: usize) -> Result<NodeHash, EncodedTrieError> {
        let node = &self.nodes[index];
        let range = node
            .encoded_range
            .ok_or(EncodedTrieError::NonEncodedNode(index))?;
        let encoded = &self.encoded_data[range.0..range.1];
        Ok(NodeHash::from_encoded(encoded))
    }

    /// Get the cached hash of a node.
    pub fn get_hash(&self, index: usize) -> Result<NodeHash, EncodedTrieError> {
        self.hashes[index].ok_or(EncodedTrieError::NodeHashNotFound(index))
    }

    /// Assumes this node index corresponds to a leaf, and retrieves its data taking into
    /// account the overrides.
    pub fn get_leaf_data(&self, index: usize) -> Result<(Nibbles, &[u8]), RLPDecodeError> {
        let handle = &self.nodes[index].node_type;
        let NodeType::Leaf {
            partial: override_partial,
            value: override_value,
        } = handle
        else {
            panic!("not leaf in get_leaf_data");
        };

        let data = match (override_partial, override_value) {
            (Some(partial), Some(value)) => (partial.clone(), value.as_slice()),
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
            (None, None) => {
                let encoded_items = self.get_encoded_items(index)?;
                let (partial, _) = decode_bytes(encoded_items[0])?;
                let partial = Nibbles::decode_compact(partial);
                debug_assert!(partial.is_leaf());
                let (value, _) = decode_bytes(encoded_items[1])?;
                (partial, value)
            }
        };
        Ok(data)
    }

    /// Assumes this node index corresponds to an extension, and retrieves its data taking into
    /// account the overrides.
    pub fn get_extension_data(&self, index: usize) -> Result<Nibbles, RLPDecodeError> {
        let handle = &self.nodes[index].node_type;
        let NodeType::Extension {
            prefix: override_prefix,
            ..
        } = handle
        else {
            panic!("not leaf in get_leaf_data");
        };

        let data = match override_prefix {
            Some(prefix) => prefix.clone(),
            None => {
                let encoded_items = self.get_encoded_items(index)?;
                let (prefix, _) = decode_bytes(encoded_items[0])?;
                let prefix = Nibbles::decode_compact(prefix);
                debug_assert!(!prefix.is_leaf());
                prefix
            }
        };
        Ok(data)
    }

    /// Assumes this node index corresponds to an extension, and retrieves the encoded
    /// child hash.
    ///
    /// # Warning
    /// The encoded data might be outdated. This function does not take into account overrides.
    pub fn get_extension_encoded_child_hash(
        &self,
        index: usize,
    ) -> Result<NodeHash, RLPDecodeError> {
        let encoded_items = self.get_encoded_items(index)?;
        let child_hash = decode_child(encoded_items[1]);
        Ok(child_hash)
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
}

fn encode_leaf(partial: Nibbles, value: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut encoder = Encoder::new(&mut buf);
    encoder = encoder.encode_bytes(&partial.encode_compact());
    encoder = encoder.encode_bytes(value);
    encoder.finish();
    buf
}

fn encode_extension(path: Nibbles, child: NodeHash) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut encoder = Encoder::new(&mut buf);
    encoder = encoder.encode_bytes(&path.encode_compact());
    encoder = child.encode(encoder);
    encoder.finish();
    buf
}

fn encode_branch(children: [Option<NodeHash>; 16]) -> Vec<u8> {
    // optimized encoding taken from rlp.rs
    let payload_len = children.iter().fold(1, |acc, child| {
        acc + if let Some(child) = child {
            RLPEncode::length(child)
        } else {
            1
        }
    });

    let mut buf: Vec<u8> = Vec::with_capacity(payload_len + 3); // 3 byte prefix headroom

    encode_length(payload_len, &mut buf);
    for child in children.iter() {
        let Some(child) = child else {
            buf.put_u8(RLP_NULL);
            continue;
        };
        match child {
            NodeHash::Hashed(hash) => hash.0.encode(&mut buf),
            NodeHash::Inline((_, 0)) => buf.put_u8(RLP_NULL),
            NodeHash::Inline((encoded, len)) => buf.put_slice(&encoded[..*len as usize]),
        }
    }
    buf.put_u8(RLP_NULL);
    buf
}

impl TryFrom<&EthrexTrieNode> for EncodedTrie {
    type Error = RLPDecodeError;

    fn try_from(root: &EthrexTrieNode) -> Result<Self, Self::Error> {
        let mut trie = EncodedTrie::default();

        fn recursive(value: &EthrexTrieNode, trie: &mut EncodedTrie) -> Result<(), RLPDecodeError> {
            let handle = match value {
                EthrexTrieNode::Branch(node) => {
                    let mut children_indices = [None; 16];
                    for (i, choice) in node
                        .choices
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| c.is_valid())
                    {
                        match choice {
                            EthrexTrieNodeRef::Node(choice, _) => {
                                recursive(choice, trie)?;
                                children_indices[i] = Some(Some(trie.nodes.len() - 1));
                            }
                            EthrexTrieNodeRef::Hash(inline @ NodeHash::Inline(_)) => {
                                let choice = EthrexTrieNode::decode(inline.as_ref())?;
                                recursive(&choice, trie)?;
                                children_indices[i] = Some(Some(trie.nodes.len() - 1));
                            }
                            _ => children_indices[i] = Some(None),
                        }
                    }
                    NodeType::Branch { children_indices }
                }
                EthrexTrieNode::Extension(node) => {
                    let mut child_index = None;
                    match &node.child {
                        EthrexTrieNodeRef::Node(child, _) => {
                            recursive(child, trie)?;
                            child_index = Some(trie.nodes.len() - 1);
                        }
                        EthrexTrieNodeRef::Hash(inline @ NodeHash::Inline(_)) => {
                            let child = EthrexTrieNode::decode(inline.as_ref())?;
                            recursive(&child, trie)?;
                            child_index = Some(trie.nodes.len() - 1);
                        }
                        _ => {}
                    }
                    NodeType::Extension {
                        prefix: None,
                        child_index,
                    }
                }
                EthrexTrieNode::Leaf(_) => NodeType::Leaf {
                    partial: None,
                    value: None,
                },
            };

            let offset = trie.encoded_data.len();
            trie.encoded_data.extend(value.encode_to_vec());
            trie.nodes.push(Node {
                node_type: handle,
                encoded_range: Some((offset, trie.encoded_data.len())),
            });
            Ok(())
        }

        recursive(root, &mut trie)?;
        trie.root_index = Some(trie.nodes.len() - 1); // last stored node is the root
        Ok(trie)
    }
}

#[cfg(test)]
mod test {
    use proptest::{collection::vec, prelude::*};

    use super::*;
    use crate::Trie;

    const MAX_KEY_SIZE: usize = 32;
    const MAX_VALUE_SIZE: usize = 256;
    const MAX_KV_PAIRS: usize = 100;

    fn kv_pairs_strategy() -> impl Strategy<Value = (Vec<(Vec<u8>, Vec<u8>)>, Vec<usize>)> {
        // create random key-values, with keys all the same size, and a random permutation of indices
        (1usize..=MAX_KEY_SIZE).prop_flat_map(|key_len| {
            prop::collection::vec(
                (
                    vec(any::<u8>(), key_len),
                    vec(any::<u8>(), 0..MAX_VALUE_SIZE),
                ),
                1..=MAX_KV_PAIRS,
            )
            .prop_flat_map(|kvs| {
                let len = kvs.len();
                let shuffle = vec(..len, ..len).prop_shuffle();
                (Just(kvs), shuffle)
            })
        })
    }

    proptest! {
        #[test]
        fn proptest_from_compare_hash((kv, _) in kv_pairs_strategy()) {
            let mut trie = Trie::new_temp();

            for (key, value) in kv.iter(){
                trie.insert(key.clone(), value.clone()).unwrap();
            }

            let root_node = trie.get_root_node(Nibbles::default()).unwrap();
            let mut flat_trie = EncodedTrie::try_from(&(*root_node)).unwrap();

            let hash = trie.hash_no_commit();
            let flat_trie_hash = flat_trie.hash().unwrap();

            prop_assert_eq!(hash, flat_trie_hash.finalize());
        }

        #[test]
        fn proptest_insert_compare_hash((kv, _) in kv_pairs_strategy()) {
            let mut trie = Trie::new_temp();
            let mut flat_trie = EncodedTrie::default();

            for (key, value) in kv.iter(){
                trie.insert(key.clone(), value.clone()).unwrap();
                flat_trie.insert(key.clone(), value.clone()).unwrap();
                let hash = trie.hash_no_commit();
                let flat_trie_hash = flat_trie.hash().unwrap();
                prop_assert_eq!(hash, flat_trie_hash.finalize());
            }
        }

        #[test]
        fn proptest_insert_remove_compare_hash((kv, shuffle) in kv_pairs_strategy()) {
            let mut trie = Trie::new_temp();
            let mut flat_trie = EncodedTrie::default();

            for (key, value) in kv.iter() {
                trie.insert(key.clone(), value.clone()).unwrap();
                flat_trie.insert(key.clone(), value.clone()).unwrap();
                let hash = trie.hash_no_commit();
                let flat_trie_hash = flat_trie.hash().unwrap();
                prop_assert_eq!(hash, flat_trie_hash.finalize());
            }

            for i in shuffle.iter() {
                let key = &kv[*i].0;
                trie.remove(key).unwrap();
                flat_trie.remove(key).unwrap();
                let hash = trie.hash_no_commit();
                let flat_trie_hash = flat_trie.hash().unwrap();
                prop_assert_eq!(hash, flat_trie_hash.finalize());
            }
        }
    }
}
