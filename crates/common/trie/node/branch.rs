use std::io::Write;

use digest::core_api::CoreWrapper;
use ethereum_types::H256;
use ethrex_rlp::structs::Encoder;
use sha3::{Digest, Keccak256, Keccak256Core};

use crate::{TrieDB, ValueRLP, error::TrieError, nibbles::Nibbles, node_hash::NodeHash};

use super::{ExtensionNode, LeafNode, Node, NodeRef, ValueOrHash};

/// Branch Node of an an Ethereum Compatible Patricia Merkle Trie
/// Contains the node's value and the hash of its children nodes
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BranchNode {
    pub choices: [NodeRef; 16],
    pub value: ValueRLP,
}

impl BranchNode {
    const EMPTY_REF: NodeRef = NodeRef::Hash(NodeHash::Inline(([0; 31], 0)));

    /// Empty choice array for more convenient node-building
    pub const EMPTY_CHOICES: [NodeRef; 16] = [Self::EMPTY_REF; 16];

    /// Creates a new branch node given its children, without any stored value
    pub fn new(choices: [NodeRef; 16]) -> Self {
        Self {
            choices,
            value: Default::default(),
        }
    }

    /// Creates a new branch node given its children and value
    pub const fn new_with_value(choices: [NodeRef; 16], value: ValueRLP) -> Self {
        Self { choices, value }
    }

    /// Updates the node's path and value
    pub fn update(&mut self, new_value: ValueRLP) {
        self.value = new_value;
    }

    /// Retrieves a value from the subtrie originating from this node given its path
    pub fn get(&self, db: &dyn TrieDB, mut path: Nibbles) -> Result<Option<ValueRLP>, TrieError> {
        // If path is at the end, return to its own value if present.
        // Otherwise, check the corresponding choice and delegate accordingly if present.
        if let Some(choice) = path.next_choice() {
            // Delegate to children if present
            let child_ref = &self.choices[choice];
            if child_ref.is_valid() {
                let child_node = child_ref.get_node(db)?.ok_or(TrieError::InconsistentTree)?;
                child_node.get(db, path)
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
        db: &dyn TrieDB,
        mut path: Nibbles,
        value: ValueOrHash,
    ) -> Result<Node, TrieError> {
        // If path is at the end, insert or replace its own value.
        // Otherwise, check the corresponding choice and insert or delegate accordingly.
        if let Some(choice) = path.next_choice() {
            match (&mut self.choices[choice], value) {
                // Create new child (leaf node)
                (choice_ref, ValueOrHash::Value(value)) if !choice_ref.is_valid() => {
                    let new_leaf = LeafNode::new(path, value);
                    *choice_ref = Node::from(new_leaf).into();
                }
                // Insert into existing child and then update it
                (choice_ref, ValueOrHash::Value(value)) => {
                    let child_node = choice_ref
                        .get_node(db)?
                        .ok_or(TrieError::InconsistentTree)?;

                    *choice_ref = child_node.insert(db, path, value)?.into();
                }
                // Insert external node hash if there are no overrides.
                (choice_ref, value @ ValueOrHash::Hash(hash)) => {
                    if !choice_ref.is_valid() {
                        *choice_ref = hash.into();
                    } else if path.is_empty() {
                        return Err(TrieError::Verify(
                            "attempt to override proof node with external hash".to_string(),
                        ));
                    } else {
                        *choice_ref = choice_ref
                            .get_node(db)?
                            .ok_or(TrieError::InconsistentTree)?
                            .insert(db, path, value)?
                            .into();
                    }
                }
            }
        } else if let ValueOrHash::Value(value) = value {
            // Insert into self
            self.update(value);
        } else {
            todo!("handle override case (error?)")
        }

        Ok(self.into())
    }

    /// Removes a value from the subtrie originating from this node given its path
    /// Returns the new root of the subtrie (if any) and the removed value if it existed in the subtrie
    pub fn remove(
        mut self,
        db: &dyn TrieDB,
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
                let child_node = self.choices[choice_index]
                    .get_node(db)?
                    .ok_or(TrieError::InconsistentTree)?;
                // Remove value from child node
                let (child_node, old_value) = child_node.remove(db, path.clone())?;
                if let Some(child_node) = child_node {
                    // Update child node
                    self.choices[choice_index] = child_node.into();
                } else {
                    // Remove child hash if the child subtrie was removed in the process
                    self.choices[choice_index] = NodeHash::default().into();
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
                let (choice_index, child_ref) = children[0];
                let child = child_ref.get_node(db)?.ok_or(TrieError::InconsistentTree)?;
                match child {
                    // Replace self with an extension node leading to the child
                    Node::Branch(_) => ExtensionNode::new(
                        Nibbles::from_hex(vec![choice_index as u8]),
                        child_ref.clone(),
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
        let mut hasher = Keccak256::new();
        self.encode_write(&mut hasher);
        let hash = hasher.finalize();
        NodeHash::Hashed(H256::from_slice(&hash))
    }

    /// Encodes the node
    /// Assumptions:
    /// - No value
    /// - 32 byte choices
    pub fn encode_write(&self, buf: &mut impl Write) {
        let payload_len = self.choices.iter().fold(1, |payload_len, child| {
            payload_len + if child.is_valid() { 33 } else { 1 }
        });
        if payload_len < 56 {
            buf.write(&[0xc0 + payload_len as u8]);
        } else if payload_len < u8::MAX as usize { // TODO: < or <=
            buf.write(&[0xf8, payload_len as u8]);
        } else {
            // ASSUMPTION: list len will never be >u16::MAX (2 bytes len)
            buf.write(&[
                0xf9,
                ((payload_len as u16) >> 8) as u8,
                (payload_len & 0xff) as u8,
            ]);
        }

        for child in self.choices.iter() {
            match child.compute_hash() {
                NodeHash::Hashed(hash) => {
                    buf.write(&[0xa0]);
                    buf.write(&hash.0);
                }
                NodeHash::Inline(raw) if raw.1 != 0 => {
                    // ASSUMPTION: nodes would never be inlined
                    // WARN: assumption is wrong actually, but the probability of an inlined node
                    // is pretty small.
                    // TODO: fix this
                    unreachable!();
                    // buf.push(0x80 + raw.1);
                    // buf.extend_from_slice(&raw.0[..raw.1 as usize]);
                }
                _ => {
                    buf.write(&[0x80]);
                }
            }
        }

        // branch's value is empty
        buf.write(&[0x80]);
    }

    /// Encodes the node
    /// Assumptions:
    /// - No value
    /// - 32 byte choices
    pub fn encode_raw(&self) -> Vec<u8> {
        // 16 items * 33 bytes, assuming branches don't have values
        // in a state or storage trie.
        // plus a 3 byte headroom for the first prefix and payload len
        const MAX_RLP_ENCODED_SIZE: usize = 528 + 3;
        let mut buf = Vec::with_capacity(MAX_RLP_ENCODED_SIZE);

        let payload_len = self.choices.iter().fold(1, |payload_len, child| {
            payload_len + if child.is_valid() { 33 } else { 1 }
        });
        if payload_len < 56 {
            buf.push(0xc0 + payload_len as u8);
        } else if payload_len < u8::MAX as usize {
            buf.extend([0xf8, payload_len as u8]);
        } else {
            buf.extend([
                0xf9,
                ((payload_len as u16) >> 8) as u8,
                (payload_len & 0xff) as u8,
            ]);
        }

        for child in self.choices.iter() {
            match child.compute_hash() {
                NodeHash::Hashed(hash) => {
                    buf.push(0xa0);
                    buf.extend(hash.0);
                }
                NodeHash::Inline(raw) if raw.1 != 0 => {
                    unreachable!();
                    // buf.push(0x80 + raw.1);
                    // buf.extend_from_slice(&raw.0[..raw.1 as usize]);
                }
                _ => {
                    buf.push(0x80);
                }
            }
        }

        // branch's value is empty
        buf.push(0x80);

        buf
    }

    /// Traverses own subtrie until reaching the node containing `path`
    /// Appends all encoded nodes traversed to `node_path` (including self)
    /// Only nodes with encoded len over or equal to 32 bytes are included
    pub fn get_path(
        &self,
        db: &dyn TrieDB,
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
            let child_ref = &self.choices[choice];
            if child_ref.is_valid() {
                let child_node = child_ref.get_node(db)?.ok_or(TrieError::InconsistentTree)?;
                child_node.get_path(db, path, node_path)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use ethereum_types::H256;

    use super::*;

    use crate::{Trie, pmt_node};

    #[test]
    fn new() {
        let node = BranchNode::new({
            let mut choices = BranchNode::EMPTY_CHOICES;

            choices[2] = NodeHash::Hashed(H256([2; 32])).into();
            choices[5] = NodeHash::Hashed(H256([5; 32])).into();

            choices
        });

        assert_eq!(
            node.choices,
            [
                Default::default(),
                Default::default(),
                NodeHash::Hashed(H256([2; 32])).into(),
                Default::default(),
                Default::default(),
                NodeHash::Hashed(H256([5; 32])).into(),
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
        let trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0,16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![0,16] => vec![0x34, 0x56, 0x78, 0x9A] },
            }
        };

        assert_eq!(
            node.get(trie.db.as_ref(), Nibbles::from_bytes(&[0x00]))
                .unwrap(),
            Some(vec![0x12, 0x34, 0x56, 0x78]),
        );
        assert_eq!(
            node.get(trie.db.as_ref(), Nibbles::from_bytes(&[0x10]))
                .unwrap(),
            Some(vec![0x34, 0x56, 0x78, 0x9A]),
        );
    }

    #[test]
    fn get_none() {
        let trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0,16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![0,16] => vec![0x34, 0x56, 0x78, 0x9A] },
            }
        };

        assert_eq!(
            node.get(trie.db.as_ref(), Nibbles::from_bytes(&[0x20]))
                .unwrap(),
            None,
        );
    }

    #[test]
    fn insert_self() {
        let trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![0, 16] => vec![0x34, 0x56, 0x78, 0x9A] },
            }
        };
        let path = Nibbles::from_bytes(&[2]);
        let value = vec![0x3];

        let node = node
            .insert(trie.db.as_ref(), path.clone(), value.clone().into())
            .unwrap();

        assert!(matches!(node, Node::Branch(_)));
        assert_eq!(node.get(trie.db.as_ref(), path).unwrap(), Some(value));
    }

    #[test]
    fn insert_choice() {
        let trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![0, 16] => vec![0x34, 0x56, 0x78, 0x9A] },
            }
        };

        let path = Nibbles::from_bytes(&[0x20]);
        let value = vec![0x21];

        let node = node
            .insert(trie.db.as_ref(), path.clone(), value.clone().into())
            .unwrap();

        assert!(matches!(node, Node::Branch(_)));
        assert_eq!(node.get(trie.db.as_ref(), path).unwrap(), Some(value));
    }

    #[test]
    fn insert_passthrough() {
        let trie = Trie::new_temp();
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
            .insert(trie.db.as_ref(), path.clone(), value.clone().into())
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
        let trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x00] },
                1 => leaf { vec![0, 16] => vec![0x10] },
            }
        };

        let (node, value) = node
            .remove(trie.db.as_ref(), Nibbles::from_bytes(&[0x00]))
            .unwrap();

        assert!(matches!(node, Some(Node::Leaf(_))));
        assert_eq!(value, Some(vec![0x00]));
    }

    #[test]
    fn remove_choice() {
        let trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x00] },
                1 => leaf { vec![0, 16] => vec![0x10] },
                2 => leaf { vec![0, 16] => vec![0x10] },
            }
        };

        let (node, value) = node
            .remove(trie.db.as_ref(), Nibbles::from_bytes(&[0x00]))
            .unwrap();

        assert!(matches!(node, Some(Node::Branch(_))));
        assert_eq!(value, Some(vec![0x00]));
    }

    #[test]
    fn remove_choice_into_value() {
        let trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x00] },
            } with_leaf { &[0x01] => vec![0xFF] }
        };

        let (node, value) = node
            .remove(trie.db.as_ref(), Nibbles::from_bytes(&[0x00]))
            .unwrap();

        assert!(matches!(node, Some(Node::Leaf(_))));
        assert_eq!(value, Some(vec![0x00]));
    }

    #[test]
    fn remove_value_into_inner() {
        let trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x00] },
            } with_leaf { &[0x1] => vec![0xFF] }
        };

        let (node, value) = node
            .remove(trie.db.as_ref(), Nibbles::from_bytes(&[]))
            .unwrap();

        assert!(matches!(node, Some(Node::Leaf(_))));
        assert_eq!(value, Some(vec![0xFF]));
    }

    #[test]
    fn remove_value() {
        let trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x00] },
                1 => leaf { vec![0, 16] => vec![0x10] },
            } with_leaf { &[0x1] => vec![0xFF] }
        };

        let (node, value) = node
            .remove(trie.db.as_ref(), Nibbles::from_bytes(&[]))
            .unwrap();

        assert!(matches!(node, Some(Node::Branch(_))));
        assert_eq!(value, Some(vec![0xFF]));
    }

    #[test]
    fn compute_hash_two_choices() {
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

    #[test]
    fn symmetric_encoding_a() {
        let node: Node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0,16] => vec![0x12, 0x34, 0x56, 0x78] },
                1 => leaf { vec![0,16] => vec![0x34, 0x56, 0x78, 0x9A] },
            }
        }
        .into();
        assert_eq!(Node::decode_raw(&node.encode_raw()).unwrap(), node)
    }

    #[test]
    fn symmetric_encoding_b() {
        let node: Node = pmt_node! { @(trie)
            branch {
                0 => leaf { vec![0, 16] => vec![0x00] },
                1 => leaf { vec![0, 16] => vec![0x10] },
                3 => extension { [0], branch {
                    0 => leaf { vec![16] => vec![0x01, 0x00] },
                    1 => leaf { vec![16] => vec![0x01, 0x01] },
                } },
            }
        }
        .into();
        assert_eq!(Node::decode_raw(&node.encode_raw()).unwrap(), node)
    }

    #[test]
    fn symmetric_encoding_c() {
        let node: Node = pmt_node! { @(trie)
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
        }
        .into();
        assert_eq!(Node::decode_raw(&node.encode_raw()).unwrap(), node)
    }
}
