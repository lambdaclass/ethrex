use std::mem;

use ethrex_rlp::encode::RLPEncode;

use crate::{
    TrieDB, ValueRLP,
    error::TrieError,
    nibbles::Nibbles,
    node::{BranchNode, NodeRemoveResult},
    node_hash::NodeHash,
};

use super::{ExtensionNode, Node, ValueOrHash};
/// Leaf Node of an an Ethereum Compatible Patricia Merkle Trie
/// Contains the node's hash, value & path
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Deserialize,
    rkyv::Serialize,
    rkyv::Archive,
)]
pub struct LeafNode {
    pub partial: Nibbles,
    pub value: ValueRLP,
}

impl LeafNode {
    /// Creates a new leaf node and stores the given (path, value) pair
    pub const fn new(partial: Nibbles, value: ValueRLP) -> Self {
        Self { partial, value }
    }

    /// Returns the stored value if the given path matches the stored path
    pub fn get(&self, path: Nibbles) -> Result<Option<ValueRLP>, TrieError> {
        if self.partial == path {
            Ok(Some(self.value.clone()))
        } else {
            Ok(None)
        }
    }

    /// Stores the received value and returns the new root of the subtrie previously consisting of self
    pub fn insert(&mut self, path: Nibbles, value: ValueOrHash) -> Result<Option<Node>, TrieError> {
        /* Possible flow paths:
            Leaf { SelfValue } -> Leaf { Value }
            Leaf { SelfValue } -> Extension { Branch { [Self,...] Value } }
            Leaf { SelfValue } -> Extension { Branch { [ Leaf { Value } , ... ], SelfValue} }
            Leaf { SelfValue } -> Branch { [ Leaf { Value }, Self, ... ], None}
        */
        // If the path matches the stored path, update the value and return self
        if self.partial == path {
            match value {
                ValueOrHash::Value(value) => self.value = value,
                ValueOrHash::Hash(_) => {
                    return Err(TrieError::Verify(
                        "attempt to override proof node with external hash".to_string(),
                    ));
                }
            }
            Ok(None)
        } else {
            let match_index = path.count_prefix(&self.partial);
            let self_choice_idx = self.partial.at(match_index);
            let new_leaf_choice_idx = path.at(match_index);
            self.partial = self.partial.offset(match_index + 1);

            let branch_node = if self_choice_idx == 16 {
                // Create a new leaf node and store the value in it
                // Create a new branch node with the leaf as a child and store self's value
                // Branch { [ Leaf { Value } , ... ], SelfValue}
                let mut choices = BranchNode::EMPTY_CHOICES;
                choices[new_leaf_choice_idx] = match value {
                    ValueOrHash::Value(value) => {
                        Node::from(LeafNode::new(path.offset(match_index + 1), value)).into()
                    }
                    ValueOrHash::Hash(hash) => hash.into(),
                };
                BranchNode::new_with_value(choices, mem::take(&mut self.value))
            } else if new_leaf_choice_idx == 16 {
                // Create a branch node with self as a child and store the value in the branch node
                // Branch { [Self,...], Value }
                let mut choices = BranchNode::EMPTY_CHOICES;
                let child: Node = self.take().into();
                choices[self_choice_idx] = child.into();
                BranchNode::new_with_value(
                    choices,
                    match value {
                        ValueOrHash::Value(value) => value,
                        // Value in branches don't happen in our use-case.
                        ValueOrHash::Hash(_) => todo!("handle override case (error?)"),
                    },
                )
            } else {
                // Create a new leaf node and store the path and value in it
                // Create a new branch node with the leaf and self as children
                // Branch { [ Leaf { Path, Value }, Self, ... ], None, None}
                let mut choices = BranchNode::EMPTY_CHOICES;
                choices[new_leaf_choice_idx] = match value {
                    ValueOrHash::Value(value) => {
                        Node::from(LeafNode::new(path.offset(match_index + 1), value)).into()
                    }
                    ValueOrHash::Hash(hash) => hash.into(),
                };
                let child: Node = self.take().into();
                choices[self_choice_idx] = child.into();
                BranchNode::new(choices)
            };

            let final_node = if match_index == 0 {
                branch_node.into()
            } else {
                // Create an extension node with the branch node as child
                // Extension { BranchNode }
                ExtensionNode::new(path.slice(0, match_index), Node::from(branch_node).into())
                    .into()
            };

            Ok(Some(final_node))
        }
    }

    /// Inserts a batch of sorted (path, value) pairs into the subtrie originating from this node.
    /// Returns `Some(Node)` if the leaf must be replaced (split into branch/extension).
    /// Returns `None` if only the leaf's value was updated.
    pub fn insert_batch(
        &mut self,
        db: &dyn TrieDB,
        updates: &[(Nibbles, ValueRLP)],
    ) -> Result<Option<Node>, TrieError> {
        if updates.is_empty() {
            return Ok(None);
        }

        // Insert the first element using existing logic to handle leaf splitting
        let (ref first_path, ref first_value) = updates[0];
        let new_node = self.insert(first_path.clone(), first_value.clone().into())?;

        if updates.len() == 1 {
            return Ok(new_node);
        }

        // If the leaf was replaced with a new structure, insert remaining into that
        if let Some(mut node) = new_node {
            node.insert_batch(db, &updates[1..])?;
            Ok(Some(node))
        } else {
            // Leaf was updated in place (same path). Insert remaining elements sequentially
            // since they must have different paths and will split the leaf.
            let new_node = self.insert(updates[1].0.clone(), updates[1].1.clone().into())?;
            if let Some(mut node) = new_node {
                if updates.len() > 2 {
                    node.insert_batch(db, &updates[2..])?;
                }
                Ok(Some(node))
            } else {
                // All updates have the same path - just keep updating value
                for (path, value) in &updates[2..] {
                    self.insert(path.clone(), value.clone().into())?;
                }
                Ok(None)
            }
        }
    }

    /// Removes own value if the path matches own path and returns self and the value if it was removed
    pub fn remove(
        &mut self,
        path: Nibbles,
    ) -> Result<(Option<NodeRemoveResult>, Option<ValueRLP>), TrieError> {
        Ok(if self.partial == path {
            (None, Some(mem::take(&mut self.value)))
        } else {
            (Some(NodeRemoveResult::Mutated), None)
        })
    }

    /// Computes the node's hash
    pub fn compute_hash(&self) -> NodeHash {
        self.compute_hash_no_alloc(&mut vec![])
    }

    /// Computes the node's hash, using the provided buffer
    pub fn compute_hash_no_alloc(&self, buf: &mut Vec<u8>) -> NodeHash {
        buf.clear();
        self.encode(buf);
        let hash = NodeHash::from_encoded(buf);
        buf.clear();
        hash
    }

    /// Encodes the node and appends it to `node_path` if the encoded node is 32 or more bytes long
    pub fn get_path(&self, node_path: &mut Vec<Vec<u8>>) -> Result<(), TrieError> {
        let encoded = self.encode_to_vec();
        if encoded.len() >= 32 {
            node_path.push(encoded);
        }
        Ok(())
    }

    /// Creates a new node by emptying `self` partial and its value
    ///
    /// This is a way to "consume" the node when we just have a mutable reference to it
    pub fn take(&mut self) -> Self {
        LeafNode {
            partial: self.partial.take(),
            value: mem::take(&mut self.value),
        }
    }
}

#[cfg(test)]
mod test {
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

    use super::*;
    use crate::{Trie, pmt_node};

    #[test]
    fn new() {
        let node = LeafNode::new(Default::default(), Default::default());
        assert_eq!(node.value, ValueRLP::default());
    }

    #[test]
    fn get_some() {
        let node = pmt_node! { @(trie)
            leaf { vec![1, 2, 16] => vec![0x12, 0x34, 0x56, 0x78] }
        };

        assert_eq!(
            node.get(Nibbles::from_bytes(&[0x12])).unwrap(),
            Some(vec![0x12, 0x34, 0x56, 0x78]),
        );
    }

    #[test]
    fn get_none() {
        let node = pmt_node! { @(trie)
            leaf { vec![1,2,16] => vec![0x12, 0x34, 0x56, 0x78] }
        };

        assert!(node.get(Nibbles::from_bytes(&[0x34])).unwrap().is_none());
    }

    #[test]
    fn insert_replace() {
        let mut node = pmt_node! { @(trie)
            leaf { vec![1,2,16] => vec![0x12, 0x34, 0x56, 0x78] }
        };

        assert!(
            node.insert(Nibbles::from_bytes(&[0x12]), vec![0x13].into())
                .unwrap()
                .is_none()
        );

        assert_eq!(node.value, vec![0x13]);
    }

    #[test]
    fn insert_branch() {
        let trie = Trie::new_temp();
        let mut node = pmt_node! { @(trie)
            leaf { vec![1,2,16] => vec![0x12, 0x34, 0x56, 0x78] }
        };
        let path = Nibbles::from_bytes(&[0x22]);
        let value = vec![0x23];
        let node = node.insert(path.clone(), value.clone().into()).unwrap();
        let node = match node {
            Some(Node::Branch(x)) => x,
            _ => panic!("expected a branch node"),
        };
        assert_eq!(node.get(trie.db.as_ref(), path).unwrap(), Some(value));
    }

    #[test]
    fn insert_extension_branch() {
        let trie = Trie::new_temp();
        let mut node = pmt_node! { @(trie)
            leaf { vec![1,2,16] => vec![0x12, 0x34, 0x56, 0x78] }
        };

        let path = Nibbles::from_bytes(&[0x13]);
        let value = vec![0x15];

        let node = node.insert(path.clone(), value.clone().into()).unwrap();

        assert!(matches!(node, Some(Node::Extension(_))));
        assert_eq!(
            node.unwrap().get(trie.db.as_ref(), path).unwrap(),
            Some(value)
        );
    }

    #[test]
    fn insert_extension_branch_value_self() {
        let trie = Trie::new_temp();
        let mut node = pmt_node! { @(trie)
            leaf { vec![1,2,16] => vec![0x12, 0x34, 0x56, 0x78] }
        };

        let path = Nibbles::from_bytes(&[0x12, 0x34]);
        let value = vec![0x17];

        let node = node.insert(path.clone(), value.clone().into()).unwrap();

        assert!(matches!(node, Some(Node::Extension(_))));
        assert_eq!(
            node.unwrap().get(trie.db.as_ref(), path).unwrap(),
            Some(value)
        );
    }

    #[test]
    fn insert_extension_branch_value_other() {
        let trie = Trie::new_temp();
        let mut node = pmt_node! { @(trie)
            leaf { vec![1, 2, 3, 4, 16] => vec![0x12, 0x34, 0x56, 0x78] }
        };

        let path = Nibbles::from_bytes(&[0x12]);
        let value = vec![0x17];

        let node = node.insert(path.clone(), value.clone().into()).unwrap();

        assert!(matches!(node, Some(Node::Extension(_))));
        assert_eq!(
            node.unwrap().get(trie.db.as_ref(), path).unwrap(),
            Some(value)
        );
    }

    // An insertion that returns branch [value=(x)] -> leaf (y) is not possible because of the path
    // restrictions: nibbles come in pairs. If the first nibble is different, the node will be a
    // branch but it cannot have a value. If the second nibble is different, then it'll be an
    // extension followed by a branch with value and a child.

    // Because of that, the two tests that would check those cases are neither necessary nor
    // possible.

    #[test]
    fn remove_self() {
        let mut node = LeafNode::new(
            Nibbles::from_bytes(&[0x12, 0x34]),
            vec![0x12, 0x34, 0x56, 0x78],
        );
        let (node, value) = node.remove(Nibbles::from_bytes(&[0x12, 0x34])).unwrap();

        assert!(node.is_none());
        assert_eq!(value, Some(vec![0x12, 0x34, 0x56, 0x78]));
    }

    #[test]
    fn remove_none() {
        let mut node = LeafNode::new(
            Nibbles::from_bytes(&[0x12, 0x34]),
            vec![0x12, 0x34, 0x56, 0x78],
        );

        let (node, value) = node.remove(Nibbles::from_bytes(&[0x12])).unwrap();

        assert!(node.is_some());
        assert_eq!(value, None);
    }

    #[test]
    fn compute_hash_x() {
        let node = LeafNode::new(Nibbles::from_bytes(b"key".as_ref()), b"value".to_vec());
        let node_hash_ref = node.compute_hash();
        assert_eq!(
            node_hash_ref.as_ref(),
            &[
                0xCB, 0x84, 0x20, 0x6B, 0x65, 0x79, 0x85, 0x76, 0x61, 0x6C, 0x75, 0x65
            ],
        );
    }

    #[test]
    fn compute_hash_long() {
        let node = LeafNode::new(
            Nibbles::from_bytes(b"key".as_ref()),
            b"a comparatively long value".to_vec(),
        );

        let node_hash_ref = node.compute_hash();
        assert_eq!(
            node_hash_ref.as_ref(),
            &[
                0xEB, 0x92, 0x75, 0xB3, 0xAE, 0x09, 0x3A, 0x17, 0x75, 0x7C, 0xFB, 0x42, 0xF7, 0xD5,
                0x57, 0xF9, 0xE5, 0x77, 0xBD, 0x5B, 0xEB, 0x86, 0xA8, 0x68, 0x49, 0x91, 0xA6, 0x5B,
                0x87, 0x5F, 0x80, 0x7A,
            ],
        );
    }

    #[test]
    fn symmetric_encoding_a() {
        let node: Node = LeafNode::new(
            Nibbles::from_bytes(b"key".as_ref()),
            b"a comparatively long value".to_vec(),
        )
        .into();
        assert_eq!(Node::decode(&node.encode_to_vec()).unwrap(), node)
    }

    #[test]
    fn symmetric_encoding_b() {
        let node: Node = LeafNode::new(
            Nibbles::from_bytes(&[0x12, 0x34]),
            vec![0x12, 0x34, 0x56, 0x78],
        )
        .into();
        assert_eq!(Node::decode(&node.encode_to_vec()).unwrap(), node)
    }

    #[test]
    fn symmetric_encoding_c() {
        let node: Node = LeafNode::new(
            Nibbles::from_bytes(&[]),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 20],
        )
        .into();
        assert_eq!(Node::decode(&node.encode_to_vec()).unwrap(), node)
    }
}
