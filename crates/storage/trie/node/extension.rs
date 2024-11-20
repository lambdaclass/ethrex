use ethrex_rlp::structs::Encoder;

use crate::error::TrieError;
use crate::nibbles::Nibbles;
use crate::node_hash::NodeHash;
use crate::state::TrieState;
use crate::ValueRLP;

use super::{BranchNode, Node};

/// Extension Node of an an Ethereum Compatible Patricia Merkle Trie
/// Contains the node's prefix and a its child node hash, doesn't store any value
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionNode {
    pub prefix: Nibbles,
    pub child: NodeHash,
}

impl ExtensionNode {
    /// Creates a new extension node given its child hash and prefix
    pub(crate) fn new(prefix: Nibbles, child: NodeHash) -> Self {
        Self { prefix, child }
    }

    /// Retrieves a value from the subtrie originating from this node given its path
    pub fn get(&self, state: &TrieState, mut path: Nibbles) -> Result<Option<ValueRLP>, TrieError> {
        // If the path is prefixed by this node's prefix, delegate to its child.
        // Otherwise, no value is present.
        if path.skip_prefix(&self.prefix) {
            let child_node = state
                .get_node(self.child.clone())?
                .expect("inconsistent internal tree structure");

            child_node.get(state, path)
        } else {
            Ok(None)
        }
    }

    /// Inserts a value into the subtrie originating from this node and returns the new root of the subtrie
    pub fn insert(
        mut self,
        state: &mut TrieState,
        path: Nibbles,
        value: ValueRLP,
    ) -> Result<Node, TrieError> {
        /* Possible flow paths:
            * Prefix fully matches path
            Extension { prefix, child } -> Extension { prefix , child' } (insert into child)
            * No match between path and prefix
            Extension { prefix, child } -> Branch { [ ] childValue } (insert into new branch node)
            Extension { prefix, child }  -> Branch { [ child ] None } (insert into new branch node)
            Extension { prefix, child }  -> Branch { [ Extension { prefix[1..], child } ] None } (insert into new branch node)
            * Prefix partially matches path
            Extension { prefix, child } -> Extension { prefix[..match], Extension { path[match..] child } } (insert into new extension node)
        */
        let match_index = path.count_prefix(&self.prefix);
        if match_index == self.prefix.len() {
            // Insert into child node
            let child_node = state
                .get_node(self.child)?
                .expect("inconsistent internal tree structure");
            let new_child_node =
                child_node.insert(state, path.offset(match_index), value.clone())?;
            self.child = new_child_node.insert_self(state)?;
            Ok(self.into())
        } else if match_index == 0 {
            let new_node = if self.prefix.len() == 1 {
                self.child
            } else {
                ExtensionNode::new(self.prefix.offset(1), self.child).insert_self(state)?
            };
            let mut choices = BranchNode::EMPTY_CHOICES;
            let branch_node = if self.prefix.at(0) == 16 {
                match state.get_node(new_node)? {
                    Some(Node::Leaf(leaf)) => {
                        BranchNode::new_with_value(Box::new(choices), leaf.value)
                    }
                    _ => panic!("inconsistent internal tree structure"),
                }
            } else {
                choices[self.prefix.at(0)] = new_node;
                BranchNode::new(Box::new(choices))
            };
            return branch_node.insert(state, path, value);
        } else {
            let new_extension = ExtensionNode::new(self.prefix.offset(match_index), self.child);
            let new_node = new_extension.insert(state, path.offset(match_index), value)?;
            self.prefix = self.prefix.slice(0, match_index);
            self.child = new_node.insert_self(state)?;
            Ok(self.into())
        }
    }

    pub fn remove(
        mut self,
        state: &mut TrieState,
        mut path: Nibbles,
    ) -> Result<(Option<Node>, Option<ValueRLP>), TrieError> {
        /* Possible flow paths:
            Extension { prefix, child } -> Extension { prefix, child } (no removal)
            Extension { prefix, child } -> None (If child.remove = None)
            Extension { prefix, child } -> Extension { prefix, ChildBranch } (if child.remove = Branch)
            Extension { prefix, child } -> ChildExtension { SelfPrefix+ChildPrefix, ChildExtensionChild } (if child.remove = Extension)
            Extension { prefix, child } -> ChildLeaf (if child.remove = Leaf)
        */

        // Check if the value is part of the child subtrie according to the prefix
        if path.skip_prefix(&self.prefix) {
            let child_node = state
                .get_node(self.child)?
                .expect("inconsistent internal tree structure");
            // Remove value from child subtrie
            let (child_node, old_value) = child_node.remove(state, path)?;
            // Restructure node based on removal
            let node = match child_node {
                // If there is no subtrie remove the node
                None => None,
                Some(node) => Some(match node {
                    // If it is a branch node set it as self's child
                    Node::Branch(branch_node) => {
                        self.child = branch_node.insert_self(state)?;
                        self.into()
                    }
                    // If it is an extension replace self with it after updating its prefix
                    Node::Extension(mut extension_node) => {
                        self.prefix.extend(&extension_node.prefix);
                        extension_node.prefix = self.prefix;
                        extension_node.into()
                    }
                    // If it is a leaf node replace self with it
                    Node::Leaf(mut leaf_node) => {
                        self.prefix.extend(&leaf_node.partial);
                        leaf_node.partial = self.prefix;
                        leaf_node.into()
                    }
                }),
            };

            Ok((node, old_value))
        } else {
            Ok((Some(self.into()), None))
        }
    }

    /// Computes the node's hash
    pub fn compute_hash(&self) -> NodeHash {
        NodeHash::from_encoded_raw(self.encode_raw())
    }

    /// Encodes the node
    pub fn encode_raw(&self) -> Vec<u8> {
        let mut buf = vec![];
        let mut encoder = Encoder::new(&mut buf).encode_bytes(&self.prefix.encode_compact());
        match &self.child {
            NodeHash::Inline(x) => {
                encoder = encoder.encode_raw(x);
            }
            NodeHash::Hashed(x) => {
                encoder = encoder.encode_bytes(&x.0);
            }
        }
        encoder.finish();
        buf
    }

    /// Inserts the node into the state and returns its hash
    pub fn insert_self(self, state: &mut TrieState) -> Result<NodeHash, TrieError> {
        let hash = self.compute_hash();
        state.insert_node(self.into(), hash.clone());
        Ok(hash)
    }

    /// Traverses own subtrie until reaching the node containing `path`
    /// Appends all encoded nodes traversed to `node_path` (including self)
    /// Only nodes with encoded len over or equal to 32 bytes are included
    pub fn get_path(
        &self,
        state: &TrieState,
        mut path: Nibbles,
        node_path: &mut Vec<Vec<u8>>,
    ) -> Result<(), TrieError> {
        // Add self to node_path (if not inlined in parent)
        let encoded = self.encode_raw();
        if encoded.len() >= 32 {
            node_path.push(encoded);
        };
        // Continue to child
        if path.skip_prefix(&self.prefix) {
            let child_node = state
                .get_node(self.child.clone())?
                .expect("inconsistent internal tree structure");
            child_node.get_path(state, path, node_path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{node::LeafNode, pmt_node, Trie};

    #[test]
    fn new() {
        let node = ExtensionNode::new(Nibbles::default(), Default::default());

        assert_eq!(node.prefix.len(), 0);
        assert_eq!(node.child, Default::default());
    }

    #[test]
    fn get_some() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            extension { [0], branch {
                0 => leaf { vec![16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![16] => vec![0x34, 0x56, 0x78, 0x9A] },
            } }
        };

        assert_eq!(
            node.get(&trie.state, Nibbles::from_bytes(&[0x00])).unwrap(),
            Some(vec![0x12, 0x34, 0x56, 0x78]),
        );
        assert_eq!(
            node.get(&trie.state, Nibbles::from_bytes(&[0x01])).unwrap(),
            Some(vec![0x34, 0x56, 0x78, 0x9A]),
        );
    }

    #[test]
    fn get_none() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            extension { [0], branch {
                0 => leaf { vec![16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![16] => vec![0x34, 0x56, 0x78, 0x9A] },
            } }
        };

        assert_eq!(
            node.get(&trie.state, Nibbles::from_bytes(&[0x02])).unwrap(),
            None,
        );
    }

    #[test]
    fn insert_passthrough() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            extension { [0], branch {
                0 => leaf { vec![16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![16] => vec![0x34, 0x56, 0x78, 0x9A] },
            } }
        };

        let node = node
            .insert(&mut trie.state, Nibbles::from_bytes(&[0x02]), vec![])
            .unwrap();
        let node = match node {
            Node::Extension(x) => x,
            _ => panic!("expected an extension node"),
        };
        assert_eq!(node.prefix.as_ref(), &[0]);
    }

    #[test]
    fn insert_branch() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            extension { [0], branch {
                0 => leaf { vec![16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![16] => vec![0x34, 0x56, 0x78, 0x9A] },
            } }
        };

        let node = node
            .insert(&mut trie.state, Nibbles::from_bytes(&[0x10]), vec![0x20])
            .unwrap();
        let node = match node {
            Node::Branch(x) => x,
            _ => panic!("expected a branch node"),
        };
        assert_eq!(
            node.get(&trie.state, Nibbles::from_bytes(&[0x10])).unwrap(),
            Some(vec![0x20])
        );
    }

    #[test]
    fn insert_branch_extension() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            extension { [0, 0], branch {
                0 => leaf { vec![16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![16]=> vec![0x34, 0x56, 0x78, 0x9A] },
            } }
        };

        let node = node
            .insert(&mut trie.state, Nibbles::from_bytes(&[0x10]), vec![0x20])
            .unwrap();
        let node = match node {
            Node::Branch(x) => x,
            _ => panic!("expected a branch node"),
        };
        assert_eq!(
            node.get(&trie.state, Nibbles::from_bytes(&[0x10])).unwrap(),
            Some(vec![0x20])
        );
    }

    #[test]
    fn insert_extension_branch() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            extension { [0, 0], branch {
                0 => leaf { vec![16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![16] => vec![0x34, 0x56, 0x78, 0x9A] },
            } }
        };

        let path = Nibbles::from_bytes(&[0x01]);
        let value = vec![0x02];

        let node = node
            .insert(&mut trie.state, path.clone(), value.clone())
            .unwrap();

        assert!(matches!(node, Node::Extension(_)));
        assert_eq!(node.get(&trie.state, path).unwrap(), Some(value));
    }

    #[test]
    fn insert_extension_branch_extension() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            extension { [0, 0], branch {
                0 => leaf { vec![16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![16] => vec![0x34, 0x56, 0x78, 0x9A] },
            } }
        };

        let path = Nibbles::from_bytes(&[0x01]);
        let value = vec![0x04];

        let node = node
            .insert(&mut trie.state, path.clone(), value.clone())
            .unwrap();

        assert!(matches!(node, Node::Extension(_)));
        assert_eq!(node.get(&trie.state, path).unwrap(), Some(value));
    }

    #[test]
    fn remove_none() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            extension { [0], branch {
                0 => leaf { vec![16] => vec![0x00] },
                1 => leaf { vec![16] => vec![0x01] },
            } }
        };

        let (node, value) = node
            .remove(&mut trie.state, Nibbles::from_bytes(&[0x02]))
            .unwrap();

        assert!(matches!(node, Some(Node::Extension(_))));
        assert_eq!(value, None);
    }

    #[test]
    fn remove_into_leaf() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            extension { [0], branch {
                0 => leaf { vec![16] => vec![0x00] },
                1 => leaf { vec![16] => vec![0x01] },
            } }
        };

        let (node, value) = node
            .remove(&mut trie.state, Nibbles::from_bytes(&[0x01]))
            .unwrap();

        assert!(matches!(node, Some(Node::Leaf(_))));
        assert_eq!(value, Some(vec![0x01]));
    }

    #[test]
    fn remove_into_extension() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            extension { [0], branch {
                0 => leaf { vec![16] => vec![0x00] },
                1 => extension { [0], branch {
                    0 => leaf { vec![16] => vec![0x01, 0x00] },
                    1 => leaf { vec![16] => vec![0x01, 0x01] },
                } },
            } }
        };

        let (node, value) = node
            .remove(&mut trie.state, Nibbles::from_bytes(&[0x00]))
            .unwrap();

        assert!(matches!(node, Some(Node::Extension(_))));
        assert_eq!(value, Some(vec![0x00]));
    }

    #[test]
    fn compute_hash() {
        /*
        Extension {
            [0, 0]
            Branch { [
               0: Leaf { [0, 16], [0x12, 0x34] }
               1: Leaf { [0, 16], [0x56, 0x78] }
            }
        }
        */
        let leaf_node_a = LeafNode::new(Nibbles::from_hex(vec![0, 16]), vec![0x12, 0x34]);
        let leaf_node_b = LeafNode::new(Nibbles::from_hex(vec![0, 16]), vec![0x56, 0x78]);
        let mut choices = BranchNode::EMPTY_CHOICES;
        choices[0] = leaf_node_a.compute_hash();
        choices[1] = leaf_node_b.compute_hash();
        let branch_node = BranchNode::new(Box::new(choices));
        let node = ExtensionNode::new(Nibbles::from_hex(vec![0, 0]), branch_node.compute_hash());

        assert_eq!(
            node.compute_hash().as_ref(),
            &[
                0xDD, 0x82, 0x00, 0x00, 0xD9, 0xC4, 0x30, 0x82, 0x12, 0x34, 0xC4, 0x30, 0x82, 0x56,
                0x78, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80,
                0x80, 0x80,
            ],
        );
    }

    #[test]
    fn compute_hash_long() {
        /*
        Extension {
            [0, 0]
            Branch { [
                0: Leaf { [0, 16], [0x12, 0x34, 0x56, 0x78, 0x9A] }
                1: Leaf { [0, 16], [0x34, 0x56, 0x78, 0x9A, 0xBC] }
            }
        }
        */
        let leaf_node_a = LeafNode::new(
            Nibbles::from_hex(vec![0, 16]),
            vec![0x12, 0x34, 0x56, 0x78, 0x9A],
        );
        let leaf_node_b = LeafNode::new(
            Nibbles::from_hex(vec![0, 16]),
            vec![0x34, 0x56, 0x78, 0x9A, 0xBC],
        );
        let mut choices = BranchNode::EMPTY_CHOICES;
        choices[0] = leaf_node_a.compute_hash();
        choices[1] = leaf_node_b.compute_hash();
        let branch_node = BranchNode::new(Box::new(choices));
        let node = ExtensionNode::new(Nibbles::from_hex(vec![0, 0]), branch_node.compute_hash());

        assert_eq!(
            node.compute_hash().as_ref(),
            &[
                0xFA, 0xBA, 0x42, 0x79, 0xB3, 0x9B, 0xCD, 0xEB, 0x7C, 0x53, 0x0F, 0xD7, 0x6E, 0x5A,
                0xA3, 0x48, 0xD3, 0x30, 0x76, 0x26, 0x14, 0x84, 0x55, 0xA0, 0xAE, 0xFE, 0x0F, 0x52,
                0x89, 0x5F, 0x36, 0x06,
            ],
        );
    }

    #[test]
    fn symmetric_encoding_a() {
        let mut trie = Trie::new_temp();
        let node: Node = pmt_node! { @(trie)
            extension { [0], branch {
                0 => leaf { vec![16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![16] => vec![0x34, 0x56, 0x78, 0x9A] },
            } }
        }
        .into();
        assert_eq!(Node::decode_raw(&node.encode_raw()).unwrap(), node)
    }

    #[test]
    fn symmetric_encoding_b() {
        let mut trie = Trie::new_temp();
        let node: Node = pmt_node! { @(trie)
            extension { [0], branch {
                0 => leaf { vec![16] => vec![0x00] },
                1 => extension { [0], branch {
                    0 => leaf { vec![16] => vec![0x01, 0x00] },
                    1 => leaf { vec![16] => vec![0x01, 0x01] },
                } },
            } }
        }
        .into();

        assert_eq!(Node::decode_raw(&node.encode_raw()).unwrap(), node)
    }

    #[test]
    fn symmetric_encoding_c() {
        let mut trie = Trie::new_temp();
        let node: Node = pmt_node! { @(trie)
            extension { [0], branch {
                0 => leaf { vec![16] => vec![0x00] },
                1 => extension { [0], branch {
                    0 => leaf { vec![16] => vec![0x01, 0x00] },
                    1 => leaf { vec![16] => vec![0x01, 0x01] },
                    2 => leaf { vec![16] => vec![0x01, 0x00] },
                    3 => leaf { vec![16] => vec![0x03, 0x01] },
                    4 => leaf { vec![16] => vec![0x04, 0x00] },
                    5 => leaf { vec![16] => vec![0x05, 0x01] },
                    6 => leaf { vec![16] => vec![0x06, 0x00] },
                    7 => leaf { vec![16] => vec![0x07, 0x01] },
                    8 => leaf { vec![16] => vec![0x08, 0x00] },
                    9 => leaf { vec![16] => vec![0x09, 0x01] },
                    10 => leaf { vec![16] => vec![0x10, 0x00] },
                    11 => leaf { vec![16] => vec![0x11, 0x01] },
                } },
            } }
        }
        .into();
        assert_eq!(Node::decode_raw(&node.encode_raw()).unwrap(), node)
    }
}
