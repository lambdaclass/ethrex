use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::{
    decode::{RLPDecode, decode_bytes}, encode::RLPEncode, error::RLPDecodeError, structs::Decoder,
};
use rkyv::with::Skip;

use crate::{Nibbles, Node, NodeRef};

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
    pub root_index: usize,
    /// Root hash that gets initialized when calling `Self::authenticate`
    #[serde(skip)]
    #[rkyv(with = Skip)]
    root_hash: Option<[u8; 32]>,
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
    pub childs: NodeChilds,
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
pub enum NodeChilds {
    /// A leaf node doesn't have any childs
    Leaf,
    /// An extension node always has a branch as a child
    Extension { child: Option<usize> },
    /// A branch node can have any node type as any of its 16 childs
    /// TODO: This can be optimized to a bitmap if the data vec is ordered (contiguous childs)
    Branch { childs: [Option<usize>; 16] },
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
                    NodeChilds::Branch { childs }
                }
                Node::Extension(node) => {
                    let mut child = None;
                    if let NodeRef::Node(child_node, _) = &node.child {
                        recursive(child_node, trie);
                        child = Some(trie.views.len() - 1);
                    }
                    NodeChilds::Extension { child }
                }
                Node::Leaf(_) => NodeChilds::Leaf,
            };

            let offset = trie.data.len();
            trie.data.extend(value.encode_to_vec());
            trie.views.push(NodeView {
                data_range: (offset, trie.data.len()),
                childs,
            });
        }

        recursive(root, &mut trie);
        trie.root_index = trie.views.len() - 1; // last stored node is the root
        trie
    }
}

impl FlatTrie {
    pub fn root_hash(&self) -> Option<[u8; 32]> {
        self.root_hash
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
        ) -> Result<Option<[u8; 32]>, RLPDecodeError> {
            match view.childs {
                NodeChilds::Leaf => {}
                NodeChilds::Extension { child } => {
                    if let Some(child) = child {
                        let Some(child_hash) = recursive(trie, &trie.views[child])? else {
                            return Ok(None);
                        };
                        let Some(items) = trie.get_encoded_items(view)? else {
                            panic!(); // TODO: err
                        };
                        let (child_data_hash, _) = decode_bytes(items[1])?;
                        if &child_hash != child_data_hash {
                            return Ok(None);
                        }
                    }
                }
                NodeChilds::Branch { childs } => {
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
                            let (child_data_hash, _) = decode_bytes(items[i])?;
                            if &child_hash != child_data_hash {
                                return Ok(None);
                            }
                        }
                    }
                }
            }
            Ok(Some(keccak_hash(trie.get_view_data(view))))
        }

        let Some(root_view) = self.views.get(self.root_index) else {
            panic!(); // TODO: err
        };
        let Some(root_hash) = recursive(&self, root_view)? else {
            return Ok(false);
        };
        self.root_hash = Some(root_hash);
        Ok(true)
    }

    pub fn get(&self, mut path: Nibbles) -> Result<Option<&[u8]>, RLPDecodeError> {
        fn recursive<'a>(
            trie: &'a FlatTrie,
            path: &mut Nibbles,
            view: &NodeView,
        ) -> Result<Option<&'a [u8]>, RLPDecodeError> {
            match view.childs {
                NodeChilds::Leaf => {
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
                NodeChilds::Extension { child } => {
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
                NodeChilds::Branch { childs } => {
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

        let Some(root_view) = self.views.get(self.root_index) else {
            panic!(); // TODO: err
        };
        recursive(&self, &mut path, root_view)
    }

    // TODO: cache decoded view?
    pub fn get_encoded_items(&self, view: &NodeView) -> Result<Option<Vec<&[u8]>>, RLPDecodeError> {
        let data = self.get_view_data(view);
        let mut decoder = Decoder::new(data)?;

        let mut rlp_items = Vec::with_capacity(17);
        while !decoder.is_done() && rlp_items.len() < 17 {
            let (item, new_decoder) = decoder.get_encoded_item_ref()?;
            decoder = new_decoder;
            rlp_items.push(item);
        }

        Ok(Some(rlp_items))
    }

    pub fn get_view_data(&self, view: &NodeView) -> &[u8] {
        &self.data[view.data_range.0..view.data_range.1]
    }
}
