use ethereum_rust_rlp::structs::Encoder;

use crate::{error::TrieError, nibbles::Nibbles, node_hash::NodeHash, state::TrieState, ValueRLP};

use super::{ExtensionNode, LeafNode, Node};

/// Branch Node of an an Ethereum Compatible Patricia Merkle Trie
/// Contains the node's value and the hash of its children nodes
#[derive(Debug, Clone)]
pub struct BranchNode {
    // TODO: check if switching to hashmap is a better solution
    pub choices: Box<[NodeHash; 16]>,
    pub value: ValueRLP,
}

impl BranchNode {
    /// Empty choice array for more convenient node-building
    pub const EMPTY_CHOICES: [NodeHash; 16] = [
        NodeHash::const_default(),
        NodeHash::const_default(),
        NodeHash::const_default(),
        NodeHash::const_default(),
        NodeHash::const_default(),
        NodeHash::const_default(),
        NodeHash::const_default(),
        NodeHash::const_default(),
        NodeHash::const_default(),
        NodeHash::const_default(),
        NodeHash::const_default(),
        NodeHash::const_default(),
        NodeHash::const_default(),
        NodeHash::const_default(),
        NodeHash::const_default(),
        NodeHash::const_default(),
    ];

    /// Creates a new branch node given its children, without any stored value
    pub fn new(choices: Box<[NodeHash; 16]>) -> Self {
        Self {
            choices,
            value: Default::default(),
        }
    }

    /// Creates a new branch node given its children and value
    pub fn new_with_value(choices: Box<[NodeHash; 16]>, value: ValueRLP) -> Self {
        Self { choices, value }
    }

    /// Updates the node's path and value
    pub fn update(&mut self, new_value: ValueRLP) {
        self.value = new_value;
    }

    /// Retrieves a value from the subtrie originating from this node given its path
    pub fn get(&self, state: &TrieState, mut path: Nibbles) -> Result<Option<ValueRLP>, TrieError> {
        // If path is at the end, return to its own value if present.
        // Otherwise, check the corresponding choice and delegate accordingly if present.
        if let Some(choice) = path.next_choice() {
            // Delegate to children if present
            let child_hash = &self.choices[choice];
            if child_hash.is_valid() {
                let child_node = state
                    .get_node(child_hash.clone())?
                    .expect("inconsistent internal tree structure");
                child_node.get(state, path)
            } else {
                Ok(None)
            }
        } else {
            // Return internal value if present.
            Ok((!self.value.is_empty()).then_some(self.value.clone()))
        }
    }

    /// Inserts a value into the subtrie originating from this node and returns the new root of the subtrie
    pub fn insert(
        mut self,
        state: &mut TrieState,
        mut path: Nibbles,
        value: ValueRLP,
    ) -> Result<Node, TrieError> {
        // If path is at the end, insert or replace its own value.
        // Otherwise, check the corresponding choice and insert or delegate accordingly.
        if let Some(choice) = path.next_choice() {
            match &mut self.choices[choice] {
                // Create new child (leaf node)
                choice_hash if !choice_hash.is_valid() => {
                    let new_leaf = LeafNode::new(path, value);
                    let child_hash = new_leaf.insert_self(state)?;
                    *choice_hash = child_hash;
                }
                // Insert into existing child and then update it
                choice_hash => {
                    let child_node = state
                        .get_node(choice_hash.clone())?
                        .expect("inconsistent internal tree structure");

                    let child_node = child_node.insert(state, path, value)?;
                    *choice_hash = child_node.insert_self(state)?;
                }
            }
        } else {
            // Insert into self
            self.update(value);
        }

        Ok(self.into())
    }

    /// Removes a value from the subtrie originating from this node given its path
    /// Returns the new root of the subtrie (if any) and the removed value if it existed in the subtrie
    pub fn remove(
        mut self,
        state: &mut TrieState,
        mut path: Nibbles,
    ) -> Result<(Option<Node>, Option<ValueRLP>), TrieError> {
        /* Possible flow paths:
            Step 1: Removal
                Branch { [ ... ] Value } -> Branch { [...], None, None } (remove from self)
                Branch { [ childA, ... ], Value } -> Branch { [childA', ... ], Value } (remove from child)

            Step 2: Restructure
                [0 children]
                Branch { [], Value } -> Leaf { Value } (no children, with value)
                Branch { [], None } -> Branch { [], None } (no children, no value)
                [1 child]
                Branch { [ ExtensionChild], _ , _ } -> Extension { ChoiceIndex+ExtensionChildPrefx, ExtensionChildChild }
                Branch { [ BranchChild ], None } -> Extension { ChoiceIndex, BranchChild }
                Branch { [ LeafChild], None } -> LeafChild
                Branch { [LeafChild], Value } -> Branch { [ LeafChild ], Value }
                [+1 children]
                Branch { [childA, childB, ... ], None } ->   Branch { [childA, childB, ... ], None }
        */

        // Step 1: Remove value
        // Check if the value is located in a child subtrie
        let value = if let Some(choice_index) = path.next_choice() {
            if self.choices[choice_index].is_valid() {
                let child_node = state
                    .get_node(self.choices[choice_index].clone())?
                    .expect("inconsistent internal tree structure");
                // Remove value from child node
                let (child_node, old_value) = child_node.remove(state, path.clone())?;
                if let Some(child_node) = child_node {
                    // Update child node
                    self.choices[choice_index] = child_node.insert_self(state)?;
                } else {
                    // Remove child hash if the child subtrie was removed in the process
                    self.choices[choice_index] = NodeHash::default();
                }
                old_value
            } else {
                None
            }
        } else {
            // Remove own value (if it has one) and return it
            if !self.value.is_empty() {
                let value = self.value;
                self.value = Default::default();

                (!value.is_empty()).then_some(value)
            } else {
                None
            }
        };

        // Step 2: Restructure self
        let children = self
            .choices
            .iter()
            .enumerate()
            .filter(|(_, child)| child.is_valid())
            .collect::<Vec<_>>();
        let new_node = match (children.len(), !self.value.is_empty()) {
            // If this node still has a value but no longer has children, convert it into a leaf node
            (0, true) => LeafNode::new(Nibbles::from_hex(vec![16]), self.value).into(),
            // If this node doesn't have a value and has only one child, replace it with its child node
            (1, false) => {
                let (choice_index, child_hash) = children[0];
                let child = state
                    .get_node(child_hash.clone())?
                    .expect("inconsistent internal tree structure");
                match child {
                    // Replace self with an extension node leading to the child
                    Node::Branch(_) => ExtensionNode::new(
                        Nibbles::from_hex(vec![choice_index as u8]),
                        child_hash.clone(),
                    )
                    .into(),
                    // Replace self with the child extension node, updating its path in the process
                    Node::Extension(mut extension_node) => {
                        extension_node.prefix.prepend(choice_index as u8);
                        extension_node.into()
                    }
                    Node::Leaf(mut leaf) => {
                        leaf.partial.prepend(choice_index as u8);
                        leaf.into()
                    }
                }
            }
            // Return the updated node
            _ => self.into(),
        };
        Ok((Some(new_node), value))
    }

    /// Computes the node's hash
    pub fn compute_hash(&self) -> NodeHash {
        NodeHash::from_encoded_raw(self.encode_raw())
    }

    /// Encodes the node
    pub fn encode_raw(&self) -> Vec<u8> {
        let mut buf = vec![];
        let mut encoder = Encoder::new(&mut buf);
        for child in self.choices.iter() {
            match child {
                NodeHash::Hashed(hash) => encoder = encoder.encode_bytes(&hash.0),
                NodeHash::Inline(raw) if !raw.is_empty() => encoder = encoder.encode_raw(raw),
                _ => encoder = encoder.encode_bytes(&[]),
            }
        }
        encoder = encoder.encode_bytes(&self.value);
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
        // Check the corresponding choice and delegate accordingly if present.
        if let Some(choice) = path.next_choice() {
            // Continue to child
            let child_hash = &self.choices[choice];
            if child_hash.is_valid() {
                let child_node = state
                    .get_node(child_hash.clone())?
                    .expect("inconsistent internal tree structure");
                child_node.get_path(state, path, node_path)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use ethereum_types::H256;

    use super::*;

    use crate::{pmt_node, Trie};

    #[test]
    fn new() {
        let node = BranchNode::new({
            let mut choices = BranchNode::EMPTY_CHOICES;

            choices[2] = NodeHash::Hashed(H256([2; 32]));
            choices[5] = NodeHash::Hashed(H256([5; 32]));

            Box::new(choices)
        });

        assert_eq!(
            *node.choices,
            [
                Default::default(),
                Default::default(),
                NodeHash::Hashed(H256([2; 32])),
                Default::default(),
                Default::default(),
                NodeHash::Hashed(H256([5; 32])),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
            ],
        );
    }

    #[test]
    fn get_some() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0,16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![0,16] => vec![0x34, 0x56, 0x78, 0x9A] },
            }
        };

        assert_eq!(
            node.get(&trie.state, Nibbles::from_bytes(&[0x00])).unwrap(),
            Some(vec![0x12, 0x34, 0x56, 0x78]),
        );
        assert_eq!(
            node.get(&trie.state, Nibbles::from_bytes(&[0x10])).unwrap(),
            Some(vec![0x34, 0x56, 0x78, 0x9A]),
        );
    }

    #[test]
    fn get_none() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0,16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![0,16] => vec![0x34, 0x56, 0x78, 0x9A] },
            }
        };

        assert_eq!(
            node.get(&trie.state, Nibbles::from_bytes(&[0x20])).unwrap(),
            None,
        );
    }

    #[test]
    fn insert_self() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![0, 16] => vec![0x34, 0x56, 0x78, 0x9A] },
            }
        };
        let path = Nibbles::from_hex(vec![2]);
        let value = vec![0x3];

        let node = node
            .insert(&mut trie.state, path.clone(), value.clone())
            .unwrap();

        assert!(matches!(node, Node::Branch(_)));
        assert_eq!(node.get(&trie.state, path).unwrap(), Some(value));
    }

    #[test]
    fn insert_choice() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![0, 16] => vec![0x34, 0x56, 0x78, 0x9A] },
            }
        };

        let path = Nibbles::from_bytes(&[0x20]);
        let value = vec![0x21];

        let node = node
            .insert(&mut trie.state, path.clone(), value.clone())
            .unwrap();

        assert!(matches!(node, Node::Branch(_)));
        assert_eq!(node.get(&trie.state, path).unwrap(), Some(value));
    }

    #[test]
    fn insert_passthrough() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![0, 16] => vec![0x34, 0x56, 0x78, 0x9A] },
            }
        };

        // The extension node is ignored since it's irrelevant in this test.
        let path = Nibbles::from_bytes(&[0x00]).offset(2);
        let value = vec![0x1];

        let new_node = node
            .clone()
            .insert(&mut trie.state, path.clone(), value.clone())
            .unwrap();

        let new_node = match new_node {
            Node::Branch(x) => x,
            _ => panic!("expected a branch node"),
        };

        assert_eq!(new_node.choices, node.choices);
        assert_eq!(new_node.value, value);
    }

    #[test]
    fn remove_choice_into_inner() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x00] },
                1 => leaf { vec![0, 16] => vec![0x10] },
            }
        };

        let (node, value) = node
            .remove(&mut trie.state, Nibbles::from_bytes(&[0x00]))
            .unwrap();

        assert!(matches!(node, Some(Node::Leaf(_))));
        assert_eq!(value, Some(vec![0x00]));
    }

    #[test]
    fn remove_choice() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x00] },
                1 => leaf { vec![0, 16] => vec![0x10] },
                2 => leaf { vec![0, 16] => vec![0x10] },
            }
        };

        let (node, value) = node
            .remove(&mut trie.state, Nibbles::from_bytes(&[0x00]))
            .unwrap();

        assert!(matches!(node, Some(Node::Branch(_))));
        assert_eq!(value, Some(vec![0x00]));
    }

    #[test]
    fn remove_choice_into_value() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x00] },
            } with_leaf { &[0x01] => vec![0xFF] }
        };

        let (node, value) = node
            .remove(&mut trie.state, Nibbles::from_bytes(&[0x00]))
            .unwrap();

        assert!(matches!(node, Some(Node::Leaf(_))));
        assert_eq!(value, Some(vec![0x00]));
    }

    #[test]
    fn remove_value_into_inner() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x00] },
            } with_leaf { &[0x1] => vec![0xFF] }
        };

        let (node, value) = node
            .remove(&mut trie.state, Nibbles::from_bytes(&[]))
            .unwrap();

        assert!(matches!(node, Some(Node::Leaf(_))));
        assert_eq!(value, Some(vec![0xFF]));
    }

    #[test]
    fn remove_value() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x00] },
                1 => leaf { vec![0, 16] => vec![0x10] },
            } with_leaf { &[0x1] => vec![0xFF] }
        };

        let (node, value) = node
            .remove(&mut trie.state, Nibbles::from_bytes(&[]))
            .unwrap();

        assert!(matches!(node, Some(Node::Branch(_))));
        assert_eq!(value, Some(vec![0xFF]));
    }

    #[test]
    fn compute_hash_two_choices() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                2 => leaf { vec![0, 16] => vec![0x20] },
                4 => leaf { vec![0, 16] => vec![0x40] },
            }
        };

        assert_eq!(
            node.compute_hash().as_ref(),
            &[
                0xD5, 0x80, 0x80, 0xC2, 0x30, 0x20, 0x80, 0xC2, 0x30, 0x40, 0x80, 0x80, 0x80, 0x80,
                0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80,
            ],
        );
    }

    #[test]
    fn compute_hash_all_choices() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0x0 => leaf { vec![0, 16] => vec![0x00] },
                0x1 => leaf { vec![0, 16] => vec![0x10] },
                0x2 => leaf { vec![0, 16] => vec![0x20] },
                0x3 => leaf { vec![0, 16] => vec![0x30] },
                0x4 => leaf { vec![0, 16] => vec![0x40] },
                0x5 => leaf { vec![0, 16] => vec![0x50] },
                0x6 => leaf { vec![0, 16] => vec![0x60] },
                0x7 => leaf { vec![0, 16] => vec![0x70] },
                0x8 => leaf { vec![0, 16] => vec![0x80] },
                0x9 => leaf { vec![0, 16] => vec![0x90] },
                0xA => leaf { vec![0, 16] => vec![0xA0] },
                0xB => leaf { vec![0, 16] => vec![0xB0] },
                0xC => leaf { vec![0, 16] => vec![0xC0] },
                0xD => leaf { vec![0, 16] => vec![0xD0] },
                0xE => leaf { vec![0, 16] => vec![0xE0] },
                0xF => leaf { vec![0, 16] => vec![0xF0] },
            }
        };

        assert_eq!(
            node.compute_hash().as_ref(),
            &[
                0x0A, 0x3C, 0x06, 0x2D, 0x4A, 0xE3, 0x61, 0xEC, 0xC4, 0x82, 0x07, 0xB3, 0x2A, 0xDB,
                0x6A, 0x3A, 0x3F, 0x3E, 0x98, 0x33, 0xC8, 0x9C, 0x9A, 0x71, 0x66, 0x3F, 0x4E, 0xB5,
                0x61, 0x72, 0xD4, 0x9D,
            ],
        );
    }

    #[test]
    fn compute_hash_one_choice_with_value() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                2 => leaf { vec![0, 16] => vec![0x20] },
                4 => leaf { vec![0, 16] => vec![0x40] },
            } with_leaf { &[0x1] => vec![0x1] }
        };

        assert_eq!(
            node.compute_hash().as_ref(),
            &[
                0xD5, 0x80, 0x80, 0xC2, 0x30, 0x20, 0x80, 0xC2, 0x30, 0x40, 0x80, 0x80, 0x80, 0x80,
                0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01,
            ],
        );
    }

    #[test]
    fn compute_hash_all_choices_with_value() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0x0 => leaf { vec![0, 16] => vec![0x00] },
                0x1 => leaf { vec![0, 16] => vec![0x10] },
                0x2 => leaf { vec![0, 16] => vec![0x20] },
                0x3 => leaf { vec![0, 16] => vec![0x30] },
                0x4 => leaf { vec![0, 16] => vec![0x40] },
                0x5 => leaf { vec![0, 16] => vec![0x50] },
                0x6 => leaf { vec![0, 16] => vec![0x60] },
                0x7 => leaf { vec![0, 16] => vec![0x70] },
                0x8 => leaf { vec![0, 16] => vec![0x80] },
                0x9 => leaf { vec![0, 16] => vec![0x90] },
                0xA => leaf { vec![0, 16] => vec![0xA0] },
                0xB => leaf { vec![0, 16] => vec![0xB0] },
                0xC => leaf { vec![0, 16] => vec![0xC0] },
                0xD => leaf { vec![0, 16] => vec![0xD0] },
                0xE => leaf { vec![0, 16] => vec![0xE0] },
                0xF => leaf { vec![0, 16] => vec![0xF0] },
            } with_leaf { &[0x1] => vec![0x1] }
        };

        assert_eq!(
            node.compute_hash().as_ref(),
            &[
                0x2A, 0x85, 0x67, 0xC5, 0x63, 0x4A, 0x87, 0xBA, 0x19, 0x6F, 0x2C, 0x65, 0x15, 0x16,
                0x66, 0x37, 0xE0, 0x9A, 0x34, 0xE6, 0xC9, 0xB0, 0x4D, 0xA5, 0x6F, 0xC4, 0x70, 0x4E,
                0x38, 0x61, 0x7D, 0x8E
            ],
        );
    }
}
