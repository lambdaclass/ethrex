use ethereum_rust_rlp::structs::Encoder;

use crate::{
    error::TrieError, nibbles::Nibbles, node::BranchNode, node_hash::NodeHash, state::TrieState,
    ValueRLP,
};

use super::{ExtensionNode, Node};
/// Leaf Node of an an Ethereum Compatible Patricia Merkle Trie
/// Contains the node's hash, value & path
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LeafNode {
    pub partial: Nibbles,
    pub value: ValueRLP,
}

impl LeafNode {
    /// Creates a new leaf node and stores the given (path, value) pair
    pub fn new(partial: Nibbles, value: ValueRLP) -> Self {
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
    pub fn insert(
        mut self,
        state: &mut TrieState,
        path: Nibbles,
        value: ValueRLP,
    ) -> Result<Node, TrieError> {
        /* Possible flow paths:
            Leaf { SelfValue } -> Leaf { Value }
            Leaf { SelfValue } -> Extension { Branch { [Self,...] Value } }
            Leaf { SelfValue } -> Extension { Branch { [ Leaf { Value } , ... ], SelfValue} }
            Leaf { SelfValue } -> Branch { [ Leaf { Value }, Self, ... ], None}
        */
        // If the path matches the stored path, update the value and return self
        if self.partial == path {
            self.value = value;
            Ok(self.into())
        } else {
            let match_index = path.count_prefix(&self.partial);
            let self_choice_idx = self.partial.at(match_index);
            let new_leaf_choice_idx = path.at(match_index);
            self.partial = self.partial.offset(match_index + 1);

            let branch_node = if self_choice_idx == 16 {
                // Create a new leaf node and store the value in it
                // Create a new branch node with the leaf as a child and store self's value
                // Branch { [ Leaf { Value } , ... ], SelfValue}
                let new_leaf = LeafNode::new(path.offset(match_index + 1), value);
                let mut choices = BranchNode::EMPTY_CHOICES;
                choices[new_leaf_choice_idx] = new_leaf.insert_self(state)?;
                BranchNode::new_with_value(Box::new(choices), self.value)
            } else if new_leaf_choice_idx == 16 {
                // Create a branch node with self as a child and store the value in the branch node
                // Branch { [Self,...], Value }
                let mut choices = BranchNode::EMPTY_CHOICES;
                choices[self_choice_idx] = self.clone().insert_self(state)?;
                BranchNode::new_with_value(Box::new(choices), value)
            } else {
                // Create a new leaf node and store the path and value in it
                // Create a new branch node with the leaf and self as children
                // Branch { [ Leaf { Path, Value }, Self, ... ], None, None}
                let new_leaf = LeafNode::new(path.offset(match_index + 1), value);
                let mut choices = BranchNode::EMPTY_CHOICES;
                choices[new_leaf_choice_idx] = new_leaf.insert_self(state)?;
                choices[self_choice_idx] = self.clone().insert_self(state)?;
                BranchNode::new(Box::new(choices))
            };

            let final_node = if match_index == 0 {
                branch_node.into()
            } else {
                // Create an extension node with the branch node as child
                // Extension { BranchNode }
                ExtensionNode::new(path.slice(0, match_index), branch_node.insert_self(state)?)
                    .into()
            };

            Ok(final_node)
        }
    }

    /// Removes own value if the path matches own path and returns self and the value if it was removed
    pub fn remove(self, path: Nibbles) -> Result<(Option<Node>, Option<ValueRLP>), TrieError> {
        Ok(if self.partial == path {
            (None, Some(self.value))
        } else {
            (Some(self.into()), None)
        })
    }

    /// Computes the node's hash
    pub fn compute_hash(&self) -> NodeHash {
        NodeHash::from_encoded_raw(self.encode_raw())
    }

    /// Encodes the node
    pub fn encode_raw(&self) -> Vec<u8> {
        let mut buf = vec![];
        Encoder::new(&mut buf)
            .encode_bytes(&self.partial.encode_compact())
            .encode_bytes(&self.value)
            .finish();
        buf
    }

    /// Inserts the node into the state and returns its hash
    /// Receives the offset that needs to be traversed to reach the leaf node from the canonical root, used to compute the node hash
    pub fn insert_self(self, state: &mut TrieState) -> Result<NodeHash, TrieError> {
        let hash = self.compute_hash();
        state.insert_node(self.into(), hash.clone());
        Ok(hash)
    }

    /// Encodes the node and appends it to `node_path` if the encoded node is 32 or more bytes long
    pub fn get_path(&self, node_path: &mut Vec<Vec<u8>>) -> Result<(), TrieError> {
        let encoded = self.encode_raw();
        if encoded.len() >= 32 {
            node_path.push(encoded);
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{pmt_node, Trie};

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
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            leaf { vec![1,2,16] => vec![0x12, 0x34, 0x56, 0x78] }
        };

        let node = node
            .insert(&mut trie.state, Nibbles::from_bytes(&[0x12]), vec![0x13])
            .unwrap();
        let node = match node {
            Node::Leaf(x) => x,
            _ => panic!("expected a leaf node"),
        };

        assert_eq!(node.value, vec![0x13]);
    }

    #[test]
    fn insert_branch() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            leaf { vec![1,2,16] => vec![0x12, 0x34, 0x56, 0x78] }
        };
        let path = Nibbles::from_bytes(&[0x22]);
        let value = vec![0x23];
        let node = node
            .insert(&mut trie.state, path.clone(), value.clone())
            .unwrap();
        let node = match node {
            Node::Branch(x) => x,
            _ => panic!("expected a branch node"),
        };
        assert_eq!(node.get(&trie.state, path).unwrap(), Some(value));
    }

    #[test]
    fn insert_extension_branch() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            leaf { vec![1,2,16] => vec![0x12, 0x34, 0x56, 0x78] }
        };

        let path = Nibbles::from_bytes(&[0x13]);
        let value = vec![0x15];

        let node = node
            .insert(&mut trie.state, path.clone(), value.clone())
            .unwrap();

        assert!(matches!(node, Node::Extension(_)));
        assert_eq!(node.get(&trie.state, path).unwrap(), Some(value));
    }

    #[test]
    fn insert_extension_branch_value_self() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            leaf { vec![1,2,16] => vec![0x12, 0x34, 0x56, 0x78] }
        };

        let path = Nibbles::from_bytes(&[0x12, 0x34]);
        let value = vec![0x17];

        let node = node
            .insert(&mut trie.state, path.clone(), value.clone())
            .unwrap();

        assert!(matches!(node, Node::Extension(_)));
        assert_eq!(node.get(&trie.state, path).unwrap(), Some(value));
    }

    #[test]
    fn insert_extension_branch_value_other() {
        let mut trie = Trie::new_temp();
        let node = pmt_node! { @(trie)
            leaf { vec![1, 2, 3, 4, 16] => vec![0x12, 0x34, 0x56, 0x78] }
        };

        let path = Nibbles::from_bytes(&[0x12]);
        let value = vec![0x17];

        let node = node
            .insert(&mut trie.state, path.clone(), value.clone())
            .unwrap();

        assert!(matches!(node, Node::Extension(_)));
        assert_eq!(node.get(&trie.state, path).unwrap(), Some(value));
    }

    // An insertion that returns branch [value=(x)] -> leaf (y) is not possible because of the path
    // restrictions: nibbles come in pairs. If the first nibble is different, the node will be a
    // branch but it cannot have a value. If the second nibble is different, then it'll be an
    // extension followed by a branch with value and a child.

    // Because of that, the two tests that would check those cases are neither necessary nor
    // possible.

    #[test]
    fn remove_self() {
        let node = LeafNode::new(
            Nibbles::from_bytes(&[0x12, 0x34]),
            vec![0x12, 0x34, 0x56, 0x78],
        );
        let (node, value) = node.remove(Nibbles::from_bytes(&[0x12, 0x34])).unwrap();

        assert!(node.is_none());
        assert_eq!(value, Some(vec![0x12, 0x34, 0x56, 0x78]));
    }

    #[test]
    fn remove_none() {
        let node = LeafNode::new(
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
            &[0xCB, 0x84, 0x20, 0x6B, 0x65, 0x79, 0x85, 0x76, 0x61, 0x6C, 0x75, 0x65],
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
        assert_eq!(Node::decode_raw(&node.encode_raw()).unwrap(), node)
    }

    #[test]
    fn symmetric_encoding_b() {
        let node: Node = LeafNode::new(
            Nibbles::from_bytes(&[0x12, 0x34]),
            vec![0x12, 0x34, 0x56, 0x78],
        )
        .into();
        assert_eq!(Node::decode_raw(&node.encode_raw()).unwrap(), node)
    }

    #[test]
    fn symmetric_encoding_c() {
        let node: Node = LeafNode::new(
            Nibbles::from_bytes(&[]),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 20],
        )
        .into();
        assert_eq!(Node::decode_raw(&node.encode_raw()).unwrap(), node)
    }
}
