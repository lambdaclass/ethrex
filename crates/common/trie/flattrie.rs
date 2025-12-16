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

use crate::{EMPTY_TRIE_HASH, Nibbles, Node, NodeHash, NodeRef, rlp::decode_child};

/// A trie implementation that is non recursive, POD and avoids copying and encoding
/// nodes by providing views into a RLP flat buffer
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
    /// Stores a contiguous byte buffer with each RLP encoded node
    pub data: Vec<u8>,
    /// Contains the structural information of the MPT
    pub views: Vec<NodeView>,
    /// The index of the view for the root of this trie
    pub root_index: Option<usize>,
    /// Root hash that gets initialized when calling `Self::authenticate`
    #[serde(skip)]
    #[rkyv(with = Skip)]
    root_hash: Option<NodeHash>,
    /// Stores new data for a view index
    #[serde(skip)]
    #[rkyv(with = Skip)]
    puts: Vec<NodeData>,
}

/// A view into a particular node
#[derive(
    Clone,
    Copy,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Serialize,
    rkyv::Deserialize,
    rkyv::Archive,
)]
pub struct NodeView {
    pub pointer: NodeViewPointer,
    /// Handle into this node's childs
    pub node_type: NodeType,
}

#[derive(
    Clone,
    Copy,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Serialize,
    rkyv::Deserialize,
    rkyv::Archive,
)]
pub enum NodeViewPointer {
    /// Indices to the RLP code of this node over the flat data buffer
    InBuffer { data_range: (usize, usize) },
    /// Index to the put vector that contains the newer data of this node
    InPut { index: usize },
}

/// Contains indices to the `handles` list for each child of a node
#[derive(
    Clone,
    Copy,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Serialize,
    rkyv::Deserialize,
    rkyv::Archive,
)]
pub enum NodeType {
    /// A leaf node doesn't have any childs
    Leaf,
    /// An extension node always has a branch as a child
    Extension { child: Option<usize> },
    /// A branch node can have any node type as any of its 16 childs
    /// TODO: This can be optimized to a bitmap if the data vec is ordered (contiguous childs)
    Branch { children: [Option<usize>; 16] },
}

#[derive(Clone)]
pub enum NodeData {
    Leaf { partial: Nibbles, value: Vec<u8> },
    Extension { path: Nibbles, child: NodeHash },
    Branch { children: [Option<NodeHash>; 16] },
}

impl From<&Node> for FlatTrie {
    fn from(root: &Node) -> Self {
        let mut trie = FlatTrie::default();

        fn recursive(value: &Node, trie: &mut FlatTrie) {
            let childs = match value {
                Node::Branch(node) => {
                    let mut childs = [None; 16];
                    for (i, choice) in node.choices.iter().enumerate() {
                        if let NodeRef::Node(choice, _) = choice {
                            recursive(&(*choice), trie);
                            childs[i] = Some(trie.views.len() - 1);
                        }
                    }
                    NodeType::Branch { children: childs }
                }
                Node::Extension(node) => {
                    let mut child = None;
                    if let NodeRef::Node(child_node, _) = &node.child {
                        recursive(child_node, trie);
                        child = Some(trie.views.len() - 1);
                    }
                    NodeType::Extension { child }
                }
                Node::Leaf(_) => NodeType::Leaf,
            };

            let offset = trie.data.len();
            trie.data.extend(value.encode_to_vec());
            trie.views.push(NodeView {
                pointer: NodeViewPointer::InBuffer {
                    data_range: (offset, trie.data.len()),
                },
                node_type: childs,
            });
        }

        recursive(root, &mut trie);
        trie.root_index = Some(trie.views.len() - 1); // last stored node is the root
        trie
    }
}

impl FlatTrie {
    // TODO: authentication should be done just once, when creating the trie.
    pub fn root_hash(&mut self) -> Result<Option<NodeHash>, RLPDecodeError> {
        if self.root_hash.is_none() && !self.authenticate()? {
            return Ok(None);
        }
        Ok(self.root_hash)
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Recursively traverses the trie, hashes each node's data and checks that their parents reference
    /// those same hashes. This way the data gets authenticated.
    ///
    /// Initializes `Self::root_hash` if successful, and returns a boolean indicating success.
    pub fn authenticate(&mut self) -> Result<bool, RLPDecodeError> {
        self.apply_puts();

        fn recursive<'a>(
            trie: &'a FlatTrie,
            view: &NodeView,
        ) -> Result<Option<NodeHash>, RLPDecodeError> {
            match view.node_type {
                NodeType::Leaf => {}
                NodeType::Extension { child } => {
                    if let Some(child) = child {
                        let Some(child_hash) = recursive(trie, &trie.views[child])? else {
                            return Ok(None);
                        };
                        let Some(items) = trie.get_encoded_items(view)? else {
                            panic!("authenticate child case"); // TODO: err
                        };
                        let child_data_hash = decode_child(items[1]);
                        if child_hash != child_data_hash {
                            return Ok(None);
                        }
                    }
                }
                NodeType::Branch { children: childs } => {
                    // TODO: we can decode just the Some(_) childs
                    let Some(items) = trie.get_encoded_items(view)? else {
                        panic!("authenticate branch case"); // TODO: err
                    };
                    for (i, child) in childs.iter().enumerate() {
                        if let Some(child) = child {
                            let child_view = &trie.views[*child];
                            let Some(child_hash) = recursive(trie, child_view)? else {
                                return Ok(None);
                            };
                            let child_data_hash = decode_child(items[i]);
                            if child_hash != child_data_hash {
                                return Ok(None);
                            }
                        }
                    }
                }
            }
            Ok(Some(trie.get_hash_view(view)))
        }

        let Some(root_view) = self.get_root_view() else {
            self.root_hash = Some((*EMPTY_TRIE_HASH).into());
            return Ok(true);
        };

        let Some(root_hash) = recursive(&self, root_view)? else {
            return Ok(false);
        };
        self.root_hash = Some(root_hash);
        Ok(true)
    }

    pub fn get(&self, path: &[u8]) -> Result<Option<&[u8]>, RLPDecodeError> {
        let mut path = Nibbles::from_bytes(path);
        fn recursive<'a>(
            trie: &'a FlatTrie,
            path: &mut Nibbles,
            view_index: usize,
        ) -> Result<Option<&'a [u8]>, RLPDecodeError> {
            let view = trie.get_view(view_index).unwrap();
            match view.node_type {
                NodeType::Leaf => {
                    let (partial, value) = trie.get_leaf_data(view_index)?;
                    if partial == *path {
                        return Ok(Some(value));
                    } else {
                        return Ok(None);
                    }
                }
                NodeType::Extension { child } => {
                    let (prefix, _) = trie.get_extension_data(view_index)?;
                    if path.skip_prefix(prefix) {
                        recursive(trie, path, child.unwrap())
                    } else {
                        Ok(None)
                    }
                }
                NodeType::Branch { children: childs } => {
                    let Some(choice) = path.next_choice() else {
                        return Ok(None);
                    };
                    let Some(child_view_index) = childs[choice] else {
                        return Ok(None);
                    };
                    recursive(trie, path, child_view_index)
                }
            }
        }

        let Some(root_index) = self.root_index else {
            return Ok(None);
        };
        recursive(&self, &mut path, root_index)
    }

    pub fn put(&mut self, node_type: NodeType, data: NodeData) -> usize {
        self.puts.push(data);
        let view = NodeView {
            pointer: NodeViewPointer::InPut {
                index: self.puts.len() - 1,
            },
            node_type,
        };
        // let start = self.data.len();
        // Self::encode(&mut self.data, data);
        // let end = self.data.len();
        // let view = NodeView {
        //     pointer: NodeViewPointer::InBuffer {
        //         data_range: (start, end),
        //     },
        //     node_type,
        // };
        self.views.push(view);
        self.views.len() - 1
    }

    pub fn apply_puts(&mut self) {
        fn recursive(trie: &mut FlatTrie, self_index: usize) {
            let view = &mut trie.views[self_index];
            if let NodeViewPointer::InPut { index } = view.pointer {
                let start = trie.data.len();
                FlatTrie::encode(&mut trie.data, trie.puts[index].clone());
                let end = trie.data.len();

                view.pointer = NodeViewPointer::InBuffer {
                    data_range: (start, end),
                };
            }

            match view.node_type {
                NodeType::Leaf => {}
                NodeType::Extension { child } => {
                    if let Some(child) = child {
                        recursive(trie, child)
                    } else {
                        dbg!("ext child not present");
                        dbg!(&self_index);
                    }
                }
                NodeType::Branch { children } => {
                    for child in children {
                        if let Some(child) = child {
                            recursive(trie, child)
                        } else {
                            dbg!("branch child not present");
                            dbg!(&self_index);
                        }
                    }
                }
            }
        }
        if let Some(root_index) = self.root_index {
            recursive(self, root_index);
        }
    }

    pub fn encode(buf: &mut impl BufMut, data: NodeData) {
        match data {
            NodeData::Leaf { partial, value } => {
                let mut encoder = Encoder::new(buf);
                encoder = encoder.encode_bytes(&partial.encode_compact());
                encoder = encoder.encode_bytes(&value);
                encoder.finish();
            }
            NodeData::Extension { path, child } => {
                let mut encoder = Encoder::new(buf);
                encoder = encoder.encode_bytes(&path.encode_compact());
                encoder = child.encode(encoder);
                encoder.finish();
            }
            NodeData::Branch { children } => {
                // optimized encoding taken from rlp.rs
                let payload_len = children.iter().fold(1, |acc, child| {
                    acc + if let Some(child) = child {
                        RLPEncode::length(child)
                    } else {
                        1
                    }
                });

                encode_length(payload_len, buf);
                for child in children.iter() {
                    let Some(child) = child else {
                        buf.put_u8(RLP_NULL);
                        continue;
                    };
                    match child {
                        NodeHash::Hashed(hash) => hash.0.encode(buf),
                        NodeHash::Inline((_, 0)) => buf.put_u8(RLP_NULL),
                        NodeHash::Inline((encoded, len)) => {
                            buf.put_slice(&encoded[..*len as usize])
                        }
                    }
                }
                buf.put_u8(RLP_NULL);
            }
        }
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
        self_view_index: usize,
        mut path: Nibbles,
        value: Vec<u8>,
    ) -> Result<usize, RLPDecodeError> {
        let self_view = self.views[self_view_index];
        match self_view.node_type {
            NodeType::Leaf => {
                let (partial, self_value) = self.get_leaf_data(self_view_index)?;
                if partial == path {
                    Ok(self.put_leaf(partial, value))
                } else {
                    // Current node will be replaced with a branch or extension node
                    let match_index = path.count_prefix(&partial);
                    let self_choice_idx = partial.at(match_index);
                    let new_leaf_choice_idx = path.at(match_index);

                    // Modify the partial of self
                    let new_self_view_index =
                        self.put_leaf(partial.offset(match_index + 1), self_value.to_vec());

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
                    let new_leaf_view_index = self.put_leaf(path.offset(match_index + 1), value);
                    let branch_view_index = self.put_branch(vec![
                        (
                            new_leaf_choice_idx,
                            (
                                Some(new_leaf_view_index),
                                self.get_hash(new_leaf_view_index),
                            ),
                        ),
                        (
                            self_choice_idx,
                            (
                                Some(new_self_view_index),
                                self.get_hash(new_self_view_index),
                            ),
                        ),
                    ]);

                    if match_index == 0 {
                        Ok(branch_view_index)
                    } else {
                        // Yields an extension node with the branch as child
                        Ok(self.put_extension(
                            path.slice(0, match_index),
                            self.get_hash(branch_view_index),
                            Some(branch_view_index),
                        ))
                    }
                }
            }
            NodeType::Extension { child } => {
                let (prefix, _) = self.get_extension_data(self_view_index)?;
                let match_index = path.count_prefix(&prefix);
                if match_index == prefix.len() {
                    let path = path.offset(match_index);
                    let new_child_view_index = self.insert_inner(
                        child.expect("missing child of extension node at match_index == prefix"),
                        path,
                        value,
                    )?;
                    Ok(self.put_extension(
                        prefix,
                        self.get_hash(new_child_view_index),
                        Some(new_child_view_index),
                    ))
                } else if match_index == 0 {
                    debug_assert!(
                        prefix.at(0) != 16,
                        "insertion into extension yielded branch with value"
                    );
                    let (_, self_child) = self.get_extension_data(self_view_index)?;
                    let branch_view_index = if prefix.len() == 1 {
                        self.put_branch(vec![(prefix.at(0), (child, self_child))])
                    } else {
                        // New extension with self_node as a child
                        let new_node_view_index =
                            self.put_extension(prefix.offset(1), self_child, child);
                        self.put_branch(vec![(
                            prefix.at(0),
                            (
                                Some(new_node_view_index),
                                self.get_hash(new_node_view_index),
                            ),
                        )])
                    };
                    self.insert_inner(branch_view_index, path, value)
                } else {
                    let (_, self_child) = self.get_extension_data(self_view_index)?;
                    let new_extension_view_index =
                        self.put_extension(prefix.offset(match_index), self_child, child);
                    let new_node_view_index = self.insert_inner(
                        new_extension_view_index,
                        path.offset(match_index),
                        value,
                    )?;

                    Ok(self.put_extension(
                        prefix.slice(0, match_index),
                        self.get_hash(new_node_view_index),
                        Some(new_node_view_index),
                    ))
                }
            }
            NodeType::Branch { mut children } => {
                let mut children_hashes = self.get_branch_data(self_view_index)?;
                let choice = path
                    .next_choice()
                    .expect("branch insertion yielded value on a branch");
                let new_child_view_index = match children[choice] {
                    None if children_hashes[choice].is_some() => {
                        panic!("Missing children of branch needed for insert")
                    }
                    None => self.put_leaf(path, value),
                    Some(view_index) => self.insert_inner(view_index, path, value)?,
                };
                children[choice] = Some(new_child_view_index);
                children_hashes[choice] = Some(self.get_hash(new_child_view_index));

                let new_children = children
                    .into_iter()
                    .zip(children_hashes.into_iter())
                    .enumerate()
                    .filter_map(|(i, (c, h))| Some((i, (c, h?))))
                    .collect();
                Ok(self.put_branch(new_children))
            }
        }
    }

    pub fn remove(&mut self, path: &[u8]) -> Result<(), RLPDecodeError> {
        let path = Nibbles::from_bytes(path);
        if let Some(root_index) = self.root_index {
            self.root_index = self.remove_inner(root_index, path)?;
        }
        self.root_hash = None;
        Ok(())
    }

    fn remove_inner(
        &mut self,
        self_view_index: usize,
        mut path: Nibbles,
    ) -> Result<Option<usize>, RLPDecodeError> {
        let self_view = self.views[self_view_index];
        match self_view.node_type {
            NodeType::Leaf => {
                let (partial, _) = self.get_leaf_data(self_view_index)?;
                if partial == path {
                    Ok(None)
                } else {
                    Ok(Some(self_view_index))
                }
            }
            NodeType::Extension { child } => {
                let (mut prefix, _) = self.get_extension_data(self_view_index)?;

                if path.skip_prefix(&prefix) {
                    let new_child_view_index = self.remove_inner(
                        child.expect("missing child of extension node at remove"),
                        path,
                    )?;
                    let Some(new_child_view_index) = new_child_view_index else {
                        return Ok(None);
                    };

                    let new_child_view = self.get_view(new_child_view_index).unwrap();
                    let new_view_index = match new_child_view.node_type {
                        NodeType::Branch { .. } => self.put_extension(
                            prefix,
                            self.get_hash(new_child_view_index),
                            Some(new_child_view_index),
                        ),
                        NodeType::Extension {
                            child: new_extension_child,
                        } => {
                            let (new_child_prefix, _) =
                                self.get_extension_data(new_child_view_index)?;
                            prefix.extend(&new_child_prefix);
                            let new_extension_child = new_extension_child
                                .expect("missing child of new extension at remove");
                            self.put_extension(
                                prefix,
                                self.get_hash(new_extension_child),
                                Some(new_extension_child),
                            )
                        }
                        NodeType::Leaf => {
                            let (partial, value) = self.get_leaf_data(new_child_view_index)?;
                            prefix.extend(&partial);
                            self.put_leaf(prefix, value.to_vec())
                        }
                    };
                    Ok(Some(new_view_index))
                } else {
                    Ok(Some(self_view_index))
                }
            }
            NodeType::Branch { mut children } => {
                let choice = path
                    .next_choice()
                    .expect("branch removal yielded value on a branch");

                let mut children_hashes = self.get_branch_data(self_view_index)?;
                let Some(child_view_index) = children[choice] else {
                    return Ok(Some(self_view_index));
                };

                let new_child_index = self.remove_inner(child_view_index, path)?;
                children[choice] = new_child_index;
                children_hashes[choice] = new_child_index.map(|i| self.get_hash(i));

                let new_children: Vec<(_, _)> = children
                    .into_iter()
                    .zip(children_hashes.into_iter())
                    .enumerate()
                    .filter_map(|(i, (c, h))| Some((i, (c, h?))))
                    .collect();

                match new_children.len() {
                    0 => Ok(None),
                    1 => {
                        let (choice_idx, (child_idx, _)) = new_children[0];
                        let child_idx = child_idx.expect("missing child of branch at remove");
                        let child_view = self
                            .get_view(child_idx)
                            .expect("missing child view of branch choice at remove");

                        match child_view.node_type {
                            NodeType::Leaf => {
                                let (mut partial, value) = self.get_leaf_data(child_view_index)?;
                                partial.prepend(choice_idx as u8);
                                Ok(Some(self.put_leaf(partial, value.to_vec())))
                            }
                            NodeType::Extension { child } => {
                                let (mut prefix, _) = self.get_extension_data(child_view_index)?;
                                prefix.prepend(choice_idx as u8);
                                let child = child
                                    .expect("missing child of extension at remove for branch case");
                                Ok(Some(self.put_extension(
                                    prefix,
                                    self.get_hash(child),
                                    Some(child),
                                )))
                            }
                            NodeType::Branch { .. } => {
                                let prefix = Nibbles::from_hex(vec![choice_idx as u8]);
                                Ok(Some(self.put_extension(
                                    prefix,
                                    self.get_hash(child_idx),
                                    Some(child_idx),
                                )))
                            }
                        }
                    }
                    _ => Ok(Some(self.put_branch(new_children))),
                }
            }
        }
    }

    /// Puts a new leaf node from a prefix and a value.
    ///
    /// Returns the new node's view index.
    pub fn put_leaf(&mut self, partial: Nibbles, value: Vec<u8>) -> usize {
        let data = NodeData::Leaf { partial, value };
        let children = NodeType::Leaf;
        self.put(children, data)
    }

    /// Puts a new extension node from a path, child hash and child view.
    ///
    /// Returns the new node's view index.
    pub fn put_extension(
        &mut self,
        path: Nibbles,
        child_hash: NodeHash,
        child_view_index: Option<usize>,
    ) -> usize {
        let data = NodeData::Extension {
            path,
            child: child_hash,
        };
        let children = NodeType::Extension {
            child: child_view_index,
        };
        self.put(children, data)
    }

    /// Puts a new branch node from a list of children in the form of (choice, view_index). Childrens
    /// need to exist in the trie's data.
    ///
    /// Returns the new node's view index.
    pub fn put_branch(&mut self, children_views: Vec<(usize, (Option<usize>, NodeHash))>) -> usize {
        let data = {
            let mut children: [_; 16] = std::array::from_fn(|_| None);
            for (choice, child) in &children_views {
                children[*choice] = Some(child.1);
            }
            NodeData::Branch { children }
        };
        let children = {
            let mut children = [None; 16];
            for (choice, child) in children_views {
                children[choice] = child.0;
            }
            NodeType::Branch { children }
        };
        self.put(children, data)
    }

    pub fn get_view(&self, index: usize) -> Option<&NodeView> {
        self.views.get(index)
    }

    pub fn get_root_view(&self) -> Option<&NodeView> {
        self.root_index
            .map(|i| self.get_view(i).expect("missing root view"))
    }

    /// Calculates the hash of a node from a view index of its data.
    /// TODO: cache? we should cache a range into the hash, because its already stored in its parent
    pub fn get_hash(&self, view_index: usize) -> NodeHash {
        dbg!("get hash");
        NodeHash::from_encoded(self.get_data(view_index))
    }

    /// Calculates the hash of a node from a view of its data.
    /// TODO: cache? we should cache a range into the hash, because its already stored in its parent
    /// TODO: consider changing into a NodeHash or similar
    pub fn get_hash_view(&self, view: &NodeView) -> NodeHash {
        dbg!("get hash view");
        NodeHash::from_encoded(self.get_data_view(view))
    }

    pub fn get_leaf_data(&self, view_index: usize) -> Result<(Nibbles, &[u8]), RLPDecodeError> {
        if let Some(put) = self.puts.get(view_index) {
            let NodeData::Leaf { partial, value } = &put else {
                panic!("called get_leaf_data with a different type of node");
            };
            Ok((partial.clone(), value.as_slice()))
        } else {
            let Some(items) = self.get_encoded_items_index(view_index)? else {
                panic!("could not get encoded items for get_leaf_data"); // TODO: err
            };

            let (partial, _) = decode_bytes(items[0])?;
            let partial = Nibbles::decode_compact(partial);
            debug_assert!(partial.is_leaf());
            let (value, _) = decode_bytes(items[1])?;

            Ok((partial, value))
        }
    }

    pub fn get_extension_data(
        &self,
        view_index: usize,
    ) -> Result<(Nibbles, NodeHash), RLPDecodeError> {
        if let Some(put) = self.puts.get(view_index) {
            let NodeData::Extension { path, child } = &put else {
                panic!("called get_extension_data with a different type of node");
            };
            Ok((path.clone(), child.clone()))
        } else {
            let Some(items) = self.get_encoded_items_index(view_index)? else {
                panic!("could not get encoded items for get_extension_data"); // TODO: err
            };

            let (prefix, _) = decode_bytes(items[0])?;
            let prefix = Nibbles::decode_compact(prefix);
            debug_assert!(!prefix.is_leaf());
            let child = decode_child(items[1]);

            Ok((prefix, child))
        }
    }

    pub fn get_branch_data(
        &self,
        view_index: usize,
    ) -> Result<[Option<NodeHash>; 16], RLPDecodeError> {
        if let Some(put) = self.puts.get(view_index) {
            let NodeData::Branch { children } = &put else {
                panic!("called get_branch_data with a different type of node");
            };
            Ok(children.clone())
        } else {
            let Some(items) = self.get_encoded_items_index(view_index)? else {
                panic!("could not get encoded items for get_branch_data"); // TODO: err
            };

            let children_hashes: [_; 16] = std::array::from_fn(|i| {
                let child = decode_child(items[i]);
                if !child.is_empty() { Some(child) } else { None }
            });

            Ok(children_hashes)
        }
    }

    // TODO: cache decoded view?
    pub fn get_encoded_items_index(
        &self,
        index: usize,
    ) -> Result<Option<Vec<&[u8]>>, RLPDecodeError> {
        dbg!("get encoded items index");
        let data = self.get_data(index);
        let mut decoder = Decoder::new(data)?;

        let mut rlp_items = Vec::with_capacity(17);
        while !decoder.is_done() && rlp_items.len() < 17 {
            let (item, new_decoder) = decoder.get_encoded_item_ref()?;
            decoder = new_decoder;
            rlp_items.push(item);
        }

        Ok(Some(rlp_items))
    }

    // TODO: cache decoded view?
    pub fn get_encoded_items(&self, view: &NodeView) -> Result<Option<Vec<&[u8]>>, RLPDecodeError> {
        let NodeViewPointer::InBuffer { data_range } = view.pointer else {
            return Ok(None);
        };
        let data = &self.data[data_range.0..data_range.1];
        let mut decoder = Decoder::new(data)?;

        let mut rlp_items = Vec::with_capacity(17);
        while !decoder.is_done() && rlp_items.len() < 17 {
            let (item, new_decoder) = decoder.get_encoded_item_ref()?;
            decoder = new_decoder;
            rlp_items.push(item);
        }

        Ok(Some(rlp_items))
    }

    pub fn get_data(&self, view_index: usize) -> &[u8] {
        dbg!("get data with index");
        dbg!(view_index);
        self.get_data_view(&self.views[view_index])
    }

    pub fn get_data_view(&self, view: &NodeView) -> &[u8] {
        let NodeViewPointer::InBuffer { data_range } = view.pointer else {
            panic!("get_data_view")
        };
        &self.data[data_range.0..data_range.1]
    }
}

// fn encode_leaf(partial: &[u8], value: &[u8]) -> Vec<u8> {
//     // TODO: fix headroom
//     let mut buf = Vec::with_capacity(partial.len() + value.len() + 3); //  3 byte headroom
//     Encoder::new(&mut buf)
//         .encode_bytes(partial)
//         .encode_bytes(value)
//         .finish();
//     buf
// }
//
// fn encode_leaf_raw(raw_partial: &[u8], value: &[u8]) -> Vec<u8> {
//     // TODO: fix headroom
//     let mut buf = Vec::with_capacity(raw_partial.len() + value.len() + 3); //  3 byte headroom
//     Encoder::new(&mut buf)
//         .encode_raw(raw_partial)
//         .encode_bytes(value)
//         .finish();
//     buf
// }
//
// fn encode_branch(choices: [Option<[u8; 32]>; 16]) -> Vec<u8> {
//     let value_len = 1;
//     let choices_len = choices.iter().fold(0, |acc, choice| {
//         acc + choice.map(|h| RLPEncode::length(&h)).unwrap_or(0)
//     });
//     let payload_len = choices_len + value_len;
//
//     let mut buf: Vec<u8> = Vec::with_capacity(choices_len + 3); // 3 byte prefix headroom
//
//     encode_length(payload_len, &mut buf);
//     for choice in choices {
//         if let Some(choice) = choice {
//             choice.encode(&mut buf);
//         } else {
//             buf.push(RLP_NULL);
//         }
//     }
//     buf
// }

#[cfg(test)]
mod test {
    use ethrex_rlp::encode::RLPEncode;
    use proptest::{
        collection::{btree_set, vec},
        prelude::*,
    };

    use crate::{Nibbles, Trie, flattrie::FlatTrie};

    fn kv_pairs_strategy() -> impl Strategy<Value = (Vec<(Vec<u8>, Vec<u8>)>, Vec<usize>)> {
        // create random key-values, with keys all the same size, and a random permutation of indices
        (1usize..32).prop_flat_map(|key_len| {
            prop::collection::vec((vec(any::<u8>(), key_len), vec(any::<u8>(), 0..256)), 1..2)
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

                prop_assert!(flat_trie.authenticate().unwrap());
                let flat_trie_hash = flat_trie.root_hash().unwrap().unwrap();

                prop_assert_eq!(hash, flat_trie_hash.finalize());
            }
        }

        #[test]
        fn proptest_insert_remove_compare_hash((kv, shuffle) in kv_pairs_strategy()) {
            let mut trie = Trie::new_temp();
            let mut flat_trie = FlatTrie::default();

            for (key, value) in kv.iter() {
                trie.insert(key.clone(), value.clone()).unwrap();
                flat_trie.insert(key.clone(), value.clone()).unwrap();

                let hash = trie.hash_no_commit();

                prop_assert!(flat_trie.authenticate().unwrap());
                let flat_trie_hash = flat_trie.root_hash().unwrap().unwrap();

                prop_assert_eq!(hash, flat_trie_hash.finalize());
            }

            for i in shuffle.iter() {
                let key = &kv[*i].0;
                trie.remove(key).unwrap();
                flat_trie.remove(key).unwrap();

                let hash = trie.hash_no_commit();

                prop_assert!(flat_trie.authenticate().unwrap());
                let flat_trie_hash = flat_trie.root_hash().unwrap().unwrap();

                prop_assert_eq!(hash, flat_trie_hash.finalize());
            }
        }
    }
}
