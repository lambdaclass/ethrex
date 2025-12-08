use std::usize;

use ethrex_rlp::encode::RLPEncode;

use crate::{Node as TraditionalNode, NodeRef as TraditionalNodeRef};

/// A trie implementation that is non recursive, POD and avoids copying and encoding
/// nodes by providing views into a RLP flat buffer
#[derive(Default, Clone)]
pub struct FlatTrie {
    /// Stores a contiguous byte buffer with each RLP encoded node
    pub data: Vec<u8>,
    /// Contains the structural information of the MPT
    pub handles: Vec<NodeView>,
}

/// A view into a particular node
#[derive(Clone, Copy)]
pub struct NodeView {
    /// Indices to the RLP code of this node over the flat data buffer
    pub data: (usize, usize),
    /// Handle into this node's childs
    pub childs: NodeChilds,
}

/// Contains indices to the `handles` list for each child of a node
#[derive(Clone, Copy)]
pub enum NodeChilds {
    /// A leaf node doesn't have any childs
    Leaf,
    /// An extension node always has a branch as a child
    Extension { child: Option<usize> },
    /// A branch node can have any node type as any of its 16 childs
    Branch { childs: [Option<usize>; 16] },
}

impl From<&TraditionalNode> for FlatTrie {
    fn from(root: &TraditionalNode) -> Self {
        let mut trie = FlatTrie::default();

        fn recursive(value: &TraditionalNode, trie: &mut FlatTrie) {
            let childs = match value {
                TraditionalNode::Branch(node) => {
                    let mut childs = [None; 16];
                    for (i, choice) in node.choices.iter().enumerate() {
                        if let TraditionalNodeRef::Node(choice, _) = choice {
                            recursive(&(*choice), trie);
                            childs[i] = Some(trie.handles.len() - 1);
                        }
                    }
                    NodeChilds::Branch { childs }
                }
                TraditionalNode::Extension(node) => {
                    let mut child = None;
                    if let TraditionalNodeRef::Node(child_node, _) = &node.child {
                        recursive(child_node, trie);
                        child = Some(trie.handles.len() - 1);
                    }
                    NodeChilds::Extension { child }
                }
                TraditionalNode::Leaf(_) => NodeChilds::Leaf,
            };

            let offset = trie.data.len();
            trie.data.extend(value.encode_to_vec());
            trie.handles.push(NodeView {
                data: (offset, trie.data.len()),
                childs,
            });
        }

        recursive(root, &mut trie);
        trie
    }
}
