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
    pub root_hash: Option<NodeHash>,
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
    /// Indices to the RLP code of this node over the flat data buffer
    pub data_range: (usize, usize),
    /// Handle into this node's childs
    pub node_type: NodeType,
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
                data_range: (offset, trie.data.len()),
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
                            panic!(); // TODO: err
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
                        panic!(); // TODO: err
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
            view: &NodeView,
        ) -> Result<Option<&'a [u8]>, RLPDecodeError> {
            match view.node_type {
                NodeType::Leaf => {
                    let Some(items) = trie.get_encoded_items(view)? else {
                        panic!(); // TODO: err
                    };

                    let (partial, _) = decode_bytes(items[0])?;
                    let partial = Nibbles::decode_compact(partial);
                    debug_assert!(partial.is_leaf());

                    if partial == *path {
                        let (value, _) = decode_bytes(items[1])?;
                        return Ok(Some(value));
                    } else {
                        return Ok(None);
                    }
                }
                NodeType::Extension { child } => {
                    let Some(items) = trie.get_encoded_items(view)? else {
                        panic!(); // TODO: err
                    };

                    let (prefix, _) = decode_bytes(items[0])?;
                    let prefix = Nibbles::decode_compact(prefix);
                    debug_assert!(!prefix.is_leaf());

                    if path.skip_prefix(prefix) {
                        recursive(trie, path, &trie.views[child.unwrap()])
                    } else {
                        Ok(None)
                    }
                }
                NodeType::Branch { children: childs } => {
                    if let Some(choice) = path.next_choice() {
                        let Some(child_view_index) = childs[choice] else {
                            return Ok(None);
                        };
                        let child_view = trie.views[child_view_index];
                        recursive(trie, path, &child_view)
                    } else {
                        let Some(items) = trie.get_encoded_items(view)? else {
                            panic!(); // TODO: err
                        };
                        let (value, _) = decode_bytes(items[16])?;
                        Ok((!value.is_empty()).then_some(value))
                    }
                }
            }
        }

        let Some(root_view) = self.get_root_view() else {
            panic!(); // TODO: err
        };
        recursive(&self, &mut path, root_view)
    }

    pub fn put(&mut self, node_type: NodeType, data: NodeData) -> usize {
        // TODO: check that both match

        let start = self.data.len();
        let mut encoder = Encoder::new(&mut self.data);
        match data {
            NodeData::Leaf { partial, value } => {
                encoder = encoder.encode_bytes(&partial.encode_compact());
                encoder = encoder.encode_bytes(&value);
            }
            NodeData::Extension { path, child } => {
                encoder = encoder.encode_bytes(&path.encode_compact());
                encoder = child.encode(encoder);
            }
            NodeData::Branch { children } => {
                for child in children {
                    if let Some(child) = child {
                        encoder = child.encode(encoder);
                    } else {
                        encoder = encoder.encode_raw(&[RLP_NULL])
                    }
                }
                encoder = encoder.encode_raw(&[RLP_NULL])
            }
        }
        encoder.finish();
        let end = self.data.len();

        let data_range = (start, end);

        let view = NodeView {
            data_range,
            node_type,
        };
        self.views.push(view);
        self.views.len() - 1
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
                let Some(items) = self.get_encoded_items(&self_view)? else {
                    panic!();
                };

                let (partial, _) = decode_bytes(items[0])?;
                let partial = Nibbles::decode_compact(partial);
                debug_assert!(partial.is_leaf());

                if partial == path {
                    Ok(self.put_leaf(partial, value))
                } else {
                    // Current node will be replaced with a branch or extension node
                    let match_index = path.count_prefix(&partial);
                    let self_choice_idx = partial.at(match_index);
                    let new_leaf_choice_idx = path.at(match_index);

                    // Modify the partial of self
                    let (self_value, _) = decode_bytes(items[1])?;
                    let new_self_view_index =
                        self.put_leaf(partial.offset(match_index + 1), self_value.to_vec());

                    let branch_view_index = if self_choice_idx == 16 {
                        // Yields a new leaf node with the new value, and a parent branch node
                        // with the old value. We disallow branches with values.
                        unreachable!("leaf insertion yielded branch with old value");
                    } else if new_leaf_choice_idx == 16 {
                        // Yields a new branch node with the current leaf as child, and the new
                        // value in the branch. We disallow branches with values.
                        unreachable!("leaf insertion yielded branch with new value")
                    } else {
                        // Yields a new leaf with the path and value in it, and a new branch
                        // with the new and old leaf as children.
                        let new_leaf_view_index =
                            self.put_leaf(path.offset(match_index + 1), value);
                        self.put_branch(vec![
                            (new_leaf_choice_idx, new_leaf_view_index),
                            (self_choice_idx, new_self_view_index),
                        ])
                    };

                    if match_index == 0 {
                        Ok(branch_view_index)
                    } else {
                        // Yields an extension node with the branch as child
                        Ok(self.put_extension(path.slice(0, match_index), branch_view_index))
                    }
                }
            }
            NodeType::Extension { child } => {
                let Some(items) = self.get_encoded_items(&self_view)? else {
                    panic!();
                };

                let (prefix, _) = decode_bytes(items[0])?;
                let prefix = Nibbles::decode_compact(prefix);
                debug_assert!(!prefix.is_leaf());

                let match_index = path.count_prefix(&prefix);
                if match_index == prefix.len() {
                    let path = path.offset(match_index);
                    let new_child_view_index = self.insert_inner(
                        child.expect("missing child of extension node at match_index == prefix"),
                        path,
                        value,
                    )?;
                    Ok(self.put_extension(prefix, new_child_view_index))
                } else if match_index == 0 {
                    let new_node_view_index = if prefix.len() == 1 {
                        child.expect("missing child of extension node at match_index == 0")
                    } else {
                        // New extension with self_node as a child
                        let node_children = NodeType::Extension { child };
                        let child_hash = NodeHash::decode(
                            self.get_encoded_items(&self_view)?
                                .expect("missing data of self ext")[1],
                        )?;

                        let node_data = NodeData::Extension {
                            path: prefix.offset(1),
                            child: child_hash,
                        };
                        self.put(node_children, node_data)
                    };

                    let branch_view_index = if prefix.at(0) == 16 {
                        // Yields a branch with a value
                        unreachable!("extension inserting yielded a branch with a value")
                    } else {
                        // New branch with the new node as a child
                        self.put_branch(vec![(prefix.at(0), new_node_view_index)])
                    };
                    self.insert_inner(branch_view_index, path, value)
                } else {
                    let extension_children = NodeType::Extension { child };
                    let child_hash = NodeHash::decode(
                        self.get_encoded_items(&self_view)?
                            .expect("missing data of self ext")[1],
                    )?;
                    let extension_data = NodeData::Extension {
                        path: prefix.offset(match_index),
                        child: child_hash,
                    };
                    let new_extension_view_index = self.put(extension_children, extension_data);
                    let new_node_view_index = self.insert_inner(
                        new_extension_view_index,
                        path.offset(match_index),
                        value,
                    )?;

                    Ok(self.put_extension(prefix.slice(0, match_index), new_node_view_index))
                }
            }
            NodeType::Branch { mut children } => {
                // let Some(items) = self.get_encoded_items(&self_view)? else {
                //     panic!();
                // };

                // let children: [_; 16] = std::array::from_fn(|i| {
                //     let (child, _) = decode_bytes(items[i]).unwrap();
                //     if child.is_empty() {
                //         None
                //     } else {
                //         Some(child.to_vec())
                //     }
                // });

                if let Some(choice) = path.next_choice() {
                    let new_child_view_index = match children[choice] {
                        // TODO: we are not differentiating between a child which is not included in the data
                        // vs an empty choice. We should decode the RLP and get the information from there,
                        // or it could be added to the view.
                        None => self.put_leaf(path, value),
                        Some(view_index) => self.insert_inner(view_index, path, value)?,
                    };
                    children[choice] = Some(new_child_view_index);
                    let children = children
                        .into_iter()
                        .enumerate()
                        .filter_map(|(i, child)| Some((i, child?)))
                        .collect();
                    Ok(self.put_branch(children))
                } else {
                    // We disallow values in branchs
                    unreachable!("wanted to insert value in a branch");
                }
            }
        }
    }

    pub fn remove(&mut self, path: Vec<u8>) -> Result<(), RLPDecodeError> {
        let path = Nibbles::from_bytes(&path);
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
                let Some(items) = self.get_encoded_items(&self_view)? else {
                    panic!();
                };

                let (partial, _) = decode_bytes(items[0])?;
                let partial = Nibbles::decode_compact(partial);
                debug_assert!(partial.is_leaf());

                if partial == path {
                    Ok(None)
                } else {
                    Ok(Some(self_view_index))
                }
            }
            NodeType::Extension { child } => {
                let mut prefix = self.get_extension_prefix(&self_view)?;

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
                        NodeType::Branch { .. } => self.put_extension(prefix, new_child_view_index),
                        NodeType::Extension {
                            child: new_extension_child,
                        } => {
                            let new_child_prefix = self.get_extension_prefix(new_child_view)?;
                            prefix.extend(&new_child_prefix);
                            self.put_extension(
                                prefix,
                                new_extension_child
                                    .expect("missing child of new extension at remove"),
                            )
                        }
                        NodeType::Leaf => {
                            let (partial, value) =
                                self.get_leaf_partial_and_value(new_child_view)?;
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
                let Some(choice) = path.next_choice() else {
                    // We disallow values on branches
                    unreachable!();
                };
                let Some(child_view_index) = children[choice] else {
                    return Ok(Some(self_view_index));
                };

                let new_child_index = self.remove_inner(child_view_index, path)?;
                children[choice] = new_child_index;

                let children: Vec<(usize, usize)> = children
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, child)| Some((i, child?)))
                    .collect();

                match children.len() {
                    0 => Ok(None),
                    1 => {
                        let (choice_idx, child_idx) = children[0];
                        let child_view = self
                            .get_view(child_idx)
                            .expect("missing child view of branch choice at remove");

                        match child_view.node_type {
                            NodeType::Leaf => {
                                let (mut partial, value) =
                                    self.get_leaf_partial_and_value(child_view)?;
                                partial.prepend(choice_idx as u8);
                                Ok(Some(self.put_leaf(partial, value.to_vec())))
                            }
                            NodeType::Extension { child } => {
                                let mut prefix = self.get_extension_prefix(child_view)?;
                                prefix.prepend(choice_idx as u8);
                                Ok(Some(self.put_extension(
                                    prefix,
                                    child.expect(
                                        "missing child of extension at remove for branch case",
                                    ),
                                )))
                            }
                            NodeType::Branch { .. } => {
                                let prefix = Nibbles::from_hex(vec![choice_idx as u8]);
                                Ok(Some(self.put_extension(prefix, child_idx)))
                            }
                        }
                    }
                    _ => Ok(Some(self.put_branch(children))),
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

    /// Puts a new extension node from a path and a child. The child needs to exist in the trie's data.
    ///
    /// Returns the new node's view index.
    pub fn put_extension(&mut self, path: Nibbles, child_view_index: usize) -> usize {
        let data = NodeData::Extension {
            path,
            child: self.get_hash(child_view_index),
        };
        let children = NodeType::Extension {
            child: Some(child_view_index),
        };
        self.put(children, data)
    }

    /// Puts a new branch node from a list of children in the form of (choice, view_index). Childrens
    /// need to exist in the trie's data.
    ///
    /// Returns the new node's view index.
    pub fn put_branch(&mut self, children_views: Vec<(usize, usize)>) -> usize {
        let data = {
            let mut children: [_; 16] = std::array::from_fn(|_| None);
            for (choice, view_index) in &children_views {
                children[*choice] = Some(self.get_hash(*view_index));
            }
            NodeData::Branch { children }
        };
        let children = {
            let mut children = [None; 16];
            for (choice, view_index) in children_views {
                children[choice] = Some(view_index);
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
        NodeHash::from_encoded(self.get_data(view_index))
    }

    /// Calculates the hash of a node from a view of its data.
    /// TODO: cache? we should cache a range into the hash, because its already stored in its parent
    /// TODO: consider changing into a NodeHash or similar
    pub fn get_hash_view(&self, view: &NodeView) -> NodeHash {
        NodeHash::from_encoded(self.get_data_view(view))
    }

    // TODO: cache decoded view?
    pub fn get_encoded_items(&self, view: &NodeView) -> Result<Option<Vec<&[u8]>>, RLPDecodeError> {
        let data = self.get_data_view(view);
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
        self.get_data_view(&self.views[view_index])
    }

    pub fn get_data_view(&self, view: &NodeView) -> &[u8] {
        &self.data[view.data_range.0..view.data_range.1]
    }

    pub fn get_extension_prefix(&self, view: &NodeView) -> Result<Nibbles, RLPDecodeError> {
        let Some(items) = self.get_encoded_items(view)? else {
            panic!();
        };

        let (prefix, _) = decode_bytes(items[0])?;
        let prefix = Nibbles::decode_compact(prefix);
        debug_assert!(!prefix.is_leaf());
        Ok(prefix)
    }

    pub fn get_leaf_partial(&self, view: &NodeView) -> Result<Nibbles, RLPDecodeError> {
        let Some(items) = self.get_encoded_items(view)? else {
            panic!();
        };

        let (partial, _) = decode_bytes(items[0])?;
        let partial = Nibbles::decode_compact(partial);
        debug_assert!(partial.is_leaf());
        Ok(partial)
    }

    pub fn get_leaf_partial_and_value(
        &self,
        view: &NodeView,
    ) -> Result<(Nibbles, &[u8]), RLPDecodeError> {
        let Some(items) = self.get_encoded_items(view)? else {
            panic!();
        };

        let (partial, _) = decode_bytes(items[0])?;
        let partial = Nibbles::decode_compact(partial);
        debug_assert!(partial.is_leaf());

        let (value, _) = decode_bytes(items[1])?;

        Ok((partial, value))
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
    use proptest::{
        collection::{btree_set, vec},
        prelude::*,
    };

    use crate::{Trie, flattrie::FlatTrie};

    proptest! {
        #[test]
        fn proptest_insert_compare_hash(data in btree_set(vec(any::<u8>(), 1), 1..100)) {
            let mut trie = Trie::new_temp();
            let mut flat_trie = FlatTrie::default();

            for val in data.iter(){
                trie.insert(val.clone(), val.clone()).unwrap();
                flat_trie.insert(val.clone(), val.clone()).unwrap();

                let hash = trie.hash_no_commit();

                prop_assert!(flat_trie.authenticate().unwrap());
                let flat_trie_hash = flat_trie.root_hash().unwrap().unwrap();
                prop_assert_eq!(hash, flat_trie_hash.finalize());
            }
        }

        #[test]
        fn proptest_insert_remove_compare_hash(data in btree_set(vec(any::<u8>(), 1), 1..100)) {
            let mut trie = Trie::new_temp();
            let mut flat_trie = FlatTrie::default();

            for val in data.iter() {
                trie.insert(val.clone(), val.clone()).unwrap();
                flat_trie.insert(val.clone(), val.clone()).unwrap();

                let hash = trie.hash_no_commit();

                prop_assert!(flat_trie.authenticate().unwrap());
                let flat_trie_hash = flat_trie.root_hash().unwrap().unwrap();
                prop_assert_eq!(hash, flat_trie_hash.finalize());
            }

            for val in data.iter().rev() {
                trie.remove(val).unwrap();
                flat_trie.remove(val.clone()).unwrap();

                let hash = trie.hash_no_commit();

                prop_assert!(flat_trie.authenticate().unwrap());
                let flat_trie_hash = flat_trie.root_hash().unwrap().unwrap();
                prop_assert_eq!(hash, flat_trie_hash.finalize());
            }
        }
    }
}
