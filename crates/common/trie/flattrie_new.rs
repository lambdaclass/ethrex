use bytes::BufMut;
use ethrex_rlp::{
    constants::RLP_NULL,
    decode::decode_bytes,
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
        /// Reference to the children.
        /// - If None, then there is no child for that choice.
        /// - If Some(None), then there is a child but its pruned.
        children_indices: [Option<Option<usize>>; 16],
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
                    recursive(trie, path, child_index.expect("pruned branch child"))
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
        let handle = &self.nodes[index].handle;
        let NodeHandle::Extension {
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

    pub fn insert(&mut self, path: Vec<u8>, value: Vec<u8>) -> Result<(), RLPDecodeError> {
        let path = Nibbles::from_bytes(&path);
        if let Some(root_index) = self.root_index {
            self.root_index = Some(self.insert_inner(root_index, path, value)?);
        } else {
            self.root_index = Some(self.put_leaf(path, value));
        }
        self.root_hash = None;
        Ok(())
    }

    fn insert_inner(
        &mut self,
        self_index: usize,
        mut path: Nibbles,
        value: Vec<u8>,
    ) -> Result<usize, RLPDecodeError> {
        let self_view = &self.nodes[self_index];
        match self_view.handle {
            NodeHandle::Leaf { .. } => {
                let (partial, _) = self.get_leaf_data(self_index)?;
                if partial == path {
                    let override_node_handle = NodeHandle::Leaf {
                        partial: None,
                        value: Some(value),
                    };
                    Ok(self.override_node(self_index, override_node_handle))
                } else {
                    // Current node will be replaced with a branch or extension node
                    let match_index = path.count_prefix(&partial);
                    let self_choice_idx = partial.at(match_index);
                    let new_leaf_choice_idx = path.at(match_index);

                    // Modify the partial of self
                    let new_self_index = self.override_node(
                        self_index,
                        NodeHandle::Leaf {
                            partial: Some(partial.offset(match_index + 1)),
                            value: None,
                        },
                    );

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
                        self.put_node(NodeHandle::Branch { children_indices })
                    };

                    if match_index == 0 {
                        Ok(branch_index)
                    } else {
                        // Yields an extension node with the branch as child
                        Ok(self.put_node(NodeHandle::Extension {
                            prefix: Some(path.slice(0, match_index)),
                            child_index: Some(branch_index),
                        }))
                    }
                }
            }
            NodeHandle::Extension { child_index, .. } => {
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
                        NodeHandle::Extension {
                            prefix: None,
                            child_index: Some(new_child_index),
                        },
                    ))
                } else if match_index == 0 {
                    debug_assert!(
                        prefix.at(0) != 16,
                        "insertion into extension yielded branch with value"
                    );
                    let branch_index = if prefix.len() == 1 {
                        let mut children_indices = [None; 16];
                        children_indices[prefix.at(0)] = Some(child_index);
                        self.put_node(NodeHandle::Branch { children_indices })
                    } else {
                        // New extension with self_node as a child
                        let new_node_index = self.put_node(NodeHandle::Extension {
                            prefix: Some(prefix.offset(1)),
                            child_index,
                        });
                        {
                            let mut children_indices = [None; 16];
                            children_indices[prefix.at(0)] = Some(Some(new_node_index));
                            self.put_node(NodeHandle::Branch { children_indices })
                        }
                    };
                    self.insert_inner(branch_index, path, value)
                } else {
                    let new_extension_index = self.override_node(
                        self_index,
                        NodeHandle::Extension {
                            prefix: Some(prefix.offset(match_index)),
                            child_index,
                        },
                    );
                    let new_node_index =
                        self.insert_inner(new_extension_index, path.offset(match_index), value)?;
                    Ok(self.put_node(NodeHandle::Extension {
                        prefix: Some(prefix.slice(0, match_index)),
                        child_index: Some(new_node_index),
                    }))
                }
            }
            NodeHandle::Branch {
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
                Ok(self.override_node(self_index, NodeHandle::Branch { children_indices }))
            }
        }
    }

    /// Adds a new node to the trie with a specific handle
    ///
    /// # Warning
    /// Handle must have all its fields initialize into Some() because there is no
    /// underlying encoded node to override.
    pub fn put_node(&mut self, handle: NodeHandle) -> usize {
        let node = Node {
            handle,
            encoded_range: None,
        };
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /// Puts a new leaf node from a prefix and a value.
    ///
    /// Returns the new node's view index.
    pub fn put_leaf(&mut self, partial: Nibbles, value: Vec<u8>) -> usize {
        let handle = NodeHandle::Leaf {
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
    pub fn override_node(&mut self, index: usize, override_node_handle: NodeHandle) -> usize {
        let original_node = self.nodes.get_mut(index).unwrap();

        let override_is_same_node_kind = matches!(
            (&original_node.handle, &override_node_handle),
            (NodeHandle::Leaf { .. }, NodeHandle::Leaf { .. })
                | (NodeHandle::Extension { .. }, NodeHandle::Extension { .. })
                | (NodeHandle::Branch { .. }, NodeHandle::Branch { .. })
        );

        // if node is not the same kind as the override, panic
        // we should use put_node() in these cases
        if !override_is_same_node_kind {
            panic!();
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

    pub fn hash(&mut self) -> Result<NodeHash, RLPDecodeError> {
        fn recursive(trie: &mut FlatTrie, index: usize) -> Result<NodeHash, RLPDecodeError> {
            let node = &trie.nodes[index];
            match &node.handle {
                NodeHandle::Leaf { partial, value } => {
                    if partial.is_some() || value.is_some() {
                        // re-encode with new values
                        let (partial, value) = trie.get_leaf_data(index)?;
                        let encoded = encode_leaf(partial, value);
                        Ok(NodeHash::from_encoded(&encoded))
                    } else {
                        // use already encoded
                        Ok(trie.hash_encoded_data(index))
                    }
                }
                NodeHandle::Extension {
                    prefix,
                    child_index,
                } => match (prefix, child_index) {
                    (None, None) => Ok(trie.hash_encoded_data(index)),
                    (_, Some(child_index)) => {
                        let child_hash = recursive(trie, *child_index)?;
                        let prefix = trie.get_extension_data(index)?;
                        let encoded = encode_extension(prefix, child_hash);
                        Ok(NodeHash::from_encoded(&encoded))
                    }
                    (Some(prefix), None) => {
                        let child_hash = trie.get_extension_encoded_child_hash(index)?;
                        let encoded = encode_extension(prefix.clone(), child_hash);
                        Ok(NodeHash::from_encoded(&encoded))
                    }
                },
                NodeHandle::Branch { children_indices } => {
                    let mut children_hashes: [Option<NodeHash>; 16] = [None; 16];
                    for (i, child) in children_indices
                        .clone()
                        .iter()
                        .enumerate()
                        .flat_map(|(i, c)| c.map(|c| (i, c)))
                    {
                        children_hashes[i] = Some(if let Some(child_index) = child {
                            recursive(trie, child_index)?
                        } else {
                            let encoded_items = trie.get_encoded_items(index)?;
                            decode_child(encoded_items[i])
                        });
                    }
                    let encoded = encode_branch(children_hashes);
                    Ok(NodeHash::from_encoded(&encoded))
                }
            }
        }
        let Some(root_index) = self.root_index else {
            return Ok((*EMPTY_TRIE_HASH).into());
        };
        recursive(self, root_index)
    }

    pub fn hash_encoded_data(&self, index: usize) -> NodeHash {
        let node = &self.nodes[index];
        let range = node.encoded_range.unwrap();
        let encoded = &self.encoded_data[range.0..range.1];
        NodeHash::from_encoded(encoded)
    }
}

fn encode_leaf(partial: Nibbles, value: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut encoder = Encoder::new(&mut buf);
    encoder = encoder.encode_bytes(&partial.encode_compact());
    encoder = encoder.encode_bytes(&value);
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
        fn proptest_insert_compare_hash((kv, _) in kv_pairs_strategy()) {
            let mut trie = Trie::new_temp();
            let mut flat_trie = FlatTrie::default();

            for (key, value) in kv.iter(){
                trie.insert(key.clone(), value.clone()).unwrap();
                flat_trie.insert(key.clone(), value.clone()).unwrap();

                let hash = trie.hash_no_commit();

                let flat_trie_hash = flat_trie.hash().unwrap();

                prop_assert_eq!(hash, flat_trie_hash.finalize());
            }
        }

        // #[test]
        // fn proptest_insert_remove_compare_hash((kv, shuffle) in kv_pairs_strategy()) {
        //     let mut trie = Trie::new_temp();
        //     let mut flat_trie = FlatTrie::default();

        //     for (key, value) in kv.iter() {
        //         trie.insert(key.clone(), value.clone()).unwrap();
        //         flat_trie.insert(key.clone(), value.clone()).unwrap();

        //         let hash = trie.hash_no_commit();

        //         let flat_trie_hash = flat_trie.hash().unwrap();

        //         prop_assert_eq!(hash, flat_trie_hash.finalize());
        //     }

        //     for i in shuffle.iter() {
        //         let key = &kv[*i].0;
        //         trie.remove(key).unwrap();
        //         flat_trie.remove(key).unwrap();

        //         let hash = trie.hash_no_commit();

        //         let flat_trie_hash = flat_trie.hash().unwrap();

        //         prop_assert_eq!(hash, flat_trie_hash.finalize());
        //     }
        // }
    }
}
