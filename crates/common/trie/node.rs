mod branch;
mod extension;
mod leaf;

use smallvec::SmallVec;
use std::{
    array,
    sync::{Arc, OnceLock},
};

pub use branch::BranchNode;
use ethrex_rlp::{
    decode::{RLPDecode, decode_bytes},
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::Decoder,
};
pub use extension::ExtensionNode;
pub use leaf::LeafNode;

use crate::{TrieDB, error::TrieError, nibbles::Nibbles};

use super::{ValueRLP, node_hash::NodeHash};

/// A reference to a node.
#[derive(Clone, Debug)]
pub enum NodeRef {
    /// The node is embedded within the reference.
    Node(Arc<Node>, OnceLock<NodeHash>),
    /// The node is in the database, referenced by its hash.
    Hash(NodeHash),
}

impl NodeRef {
    pub fn get_node(
        &self,
        prefix_len: usize,
        full_path: SmallVec<[u8; 32]>,
        db: &dyn TrieDB,
    ) -> Result<Option<Node>, TrieError> {
        match *self {
            NodeRef::Node(ref node, _) => Ok(Some(node.as_ref().clone())),
            NodeRef::Hash(NodeHash::Inline((data, len))) => {
                Ok(Some(Node::decode_raw(&data[..len as usize])?))
            }
            NodeRef::Hash(hash @ NodeHash::Hashed(_)) => db
                .get(prefix_len, full_path, hash)?
                .map(|rlp| Node::decode(&rlp).map_err(TrieError::RLPDecode))
                .transpose(),
        }
    }

    pub fn is_valid(&self) -> bool {
        match self {
            NodeRef::Node(_, _) => true,
            NodeRef::Hash(hash) => hash.is_valid(),
        }
    }

    pub fn commit(
        &mut self,
        mut prefix_len: usize,
        mut full_path: SmallVec<[u8; 32]>,
        acc: &mut Vec<(usize, SmallVec<[u8; 32]>, NodeHash, Vec<u8>)>,
    ) -> NodeHash {
        match *self {
            NodeRef::Node(ref mut node, ref mut hash) => {
                match Arc::make_mut(node) {
                    Node::Branch(node) => {
                        for (i, node) in &mut node.choices.iter_mut().enumerate() {
                            let mut full_path = full_path.clone();
                            if prefix_len % 2 == 1 {
                                let j = full_path.len() - 1;
                                full_path[j] |= i as u8;
                            } else {
                                full_path.push((i << 4) as u8);
                            }
                            node.commit(prefix_len + 1, full_path, acc);
                        }
                    }
                    Node::Extension(node) => {
                        let mut full_path = full_path.clone();
                        let mut prefix = node.prefix.clone();
                        let mut prefix_len = prefix_len;
                        while let Some(nibble) = prefix.next() {
                            if prefix_len % 2 == 1 {
                                let j = full_path.len() - 1;
                                full_path[j] |= nibble;
                            } else {
                                full_path.push(nibble << 4);
                            }
                            prefix_len += 1;
                        }
                        node.child.commit(prefix_len, full_path, acc);
                    }
                    Node::Leaf(node) => {}
                }

                let hash = hash.get_or_init(|| node.compute_hash());
                acc.push((prefix_len, full_path, *hash, node.encode_to_vec()));

                let hash = *hash;
                *self = hash.into();

                hash
            }
            NodeRef::Hash(hash) => hash,
        }
    }

    pub fn compute_hash(&self) -> NodeHash {
        match self {
            NodeRef::Node(node, hash) => *hash.get_or_init(|| node.compute_hash()),
            NodeRef::Hash(hash) => *hash,
        }
    }
}

impl Default for NodeRef {
    fn default() -> Self {
        Self::Hash(NodeHash::default())
    }
}

impl From<Node> for NodeRef {
    fn from(value: Node) -> Self {
        Self::Node(Arc::new(value), OnceLock::new())
    }
}

impl From<NodeHash> for NodeRef {
    fn from(value: NodeHash) -> Self {
        Self::Hash(value)
    }
}

impl PartialEq for NodeRef {
    fn eq(&self, other: &Self) -> bool {
        self.compute_hash() == other.compute_hash()
    }
}

pub enum ValueOrHash {
    Value(ValueRLP),
    Hash(NodeHash),
}

impl From<ValueRLP> for ValueOrHash {
    fn from(value: ValueRLP) -> Self {
        Self::Value(value)
    }
}

impl From<NodeHash> for ValueOrHash {
    fn from(value: NodeHash) -> Self {
        Self::Hash(value)
    }
}

/// A Node in an Ethereum Compatible Patricia Merkle Trie
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Branch(Box<BranchNode>),
    Extension(ExtensionNode),
    Leaf(LeafNode),
}

impl From<Box<BranchNode>> for Node {
    fn from(val: Box<BranchNode>) -> Self {
        Node::Branch(val)
    }
}

impl From<BranchNode> for Node {
    fn from(val: BranchNode) -> Self {
        Node::Branch(Box::new(val))
    }
}

impl From<ExtensionNode> for Node {
    fn from(val: ExtensionNode) -> Self {
        Node::Extension(val)
    }
}

impl From<LeafNode> for Node {
    fn from(val: LeafNode) -> Self {
        Node::Leaf(val)
    }
}

impl Node {
    /// Retrieves a value from the subtrie originating from this node given its path
    pub fn get(
        &self,
        prefix_len: usize,
        full_path: SmallVec<[u8; 32]>,
        db: &dyn TrieDB,
        path: Nibbles,
    ) -> Result<Option<ValueRLP>, TrieError> {
        match self {
            Node::Branch(n) => n.get(prefix_len, full_path, db, path),
            Node::Extension(n) => n.get(prefix_len, full_path, db, path),
            Node::Leaf(n) => n.get(prefix_len, full_path, path),
        }
    }

    /// Inserts a value into the subtrie originating from this node and returns the new root of the subtrie
    pub fn insert(
        self,
        prefix_len: usize,
        full_path: SmallVec<[u8; 32]>,
        db: &dyn TrieDB,
        path: Nibbles,
        value: impl Into<ValueOrHash>,
    ) -> Result<Node, TrieError> {
        match self {
            Node::Branch(n) => n.insert(prefix_len, full_path, db, path, value.into()),
            Node::Extension(n) => n.insert(prefix_len, full_path, db, path, value.into()),
            Node::Leaf(n) => n.insert(prefix_len, full_path, path, value.into()),
        }
    }

    /// Removes a value from the subtrie originating from this node given its path
    /// Returns the new root of the subtrie (if any) and the removed value if it existed in the subtrie
    pub fn remove(
        self,
        prefix_len: usize,
        full_path: SmallVec<[u8; 32]>,
        db: &dyn TrieDB,
        path: Nibbles,
    ) -> Result<(Option<Node>, Option<ValueRLP>), TrieError> {
        match self {
            Node::Branch(n) => n.remove(prefix_len, full_path, db, path),
            Node::Extension(n) => n.remove(prefix_len, full_path, db, path),
            Node::Leaf(n) => n.remove(prefix_len, full_path, path),
        }
    }

    /// Traverses own subtrie until reaching the node containing `path`
    /// Appends all encoded nodes traversed to `node_path` (including self)
    /// Only nodes with encoded len over or equal to 32 bytes are included
    pub fn get_path(
        &self,
        prefix_len: usize,
        full_path: SmallVec<[u8; 32]>,
        db: &dyn TrieDB,
        path: Nibbles,
        node_path: &mut Vec<Vec<u8>>,
    ) -> Result<(), TrieError> {
        match self {
            Node::Branch(n) => n.get_path(prefix_len, full_path, db, path, node_path),
            Node::Extension(n) => n.get_path(prefix_len, full_path, db, path, node_path),
            Node::Leaf(n) => n.get_path(node_path),
        }
    }

    /// Encodes the node
    pub fn encode_raw(&self) -> Vec<u8> {
        match self {
            Node::Branch(n) => n.encode_raw(),
            Node::Extension(n) => n.encode_raw(),
            Node::Leaf(n) => n.encode_raw(),
        }
    }

    /// Decodes the node
    pub fn decode_raw(rlp: &[u8]) -> Result<Self, RLPDecodeError> {
        let mut rlp_items = vec![];
        let mut decoder = Decoder::new(rlp)?;
        let mut item;
        // Get encoded fields
        loop {
            (item, decoder) = decoder.get_encoded_item()?;
            rlp_items.push(item);
            // Check if we reached the end or if we decoded more items than the ones we need
            if decoder.is_done() || rlp_items.len() > 17 {
                break;
            }
        }
        // Deserialize into node depending on the available fields
        Ok(match rlp_items.len() {
            // Leaf or Extension Node
            2 => {
                let (path, _) = decode_bytes(&rlp_items[0])?;
                let path = Nibbles::decode_compact(path);
                if path.is_leaf() {
                    // Decode as Leaf
                    let (value, _) = decode_bytes(&rlp_items[1])?;
                    LeafNode {
                        partial: path,
                        value: value.to_vec(),
                    }
                    .into()
                } else {
                    // Decode as Extension
                    ExtensionNode {
                        prefix: path,
                        child: decode_child(&rlp_items[1]).into(),
                    }
                    .into()
                }
            }
            // Branch Node
            17 => {
                let choices = array::from_fn(|i| decode_child(&rlp_items[i]).into());
                let (value, _) = decode_bytes(&rlp_items[16])?;
                BranchNode {
                    choices,
                    value: value.to_vec(),
                }
                .into()
            }
            n => {
                return Err(RLPDecodeError::Custom(format!(
                    "Invalid arg count for Node, expected 2 or 17, got {n}"
                )));
            }
        })
    }

    /// Computes the node's hash
    pub fn compute_hash(&self) -> NodeHash {
        match self {
            Node::Branch(n) => n.compute_hash(),
            Node::Extension(n) => n.compute_hash(),
            Node::Leaf(n) => n.compute_hash(),
        }
    }
}

fn decode_child(rlp: &[u8]) -> NodeHash {
    match decode_bytes(rlp) {
        Ok((hash, &[])) if hash.len() == 32 => NodeHash::from_slice(hash),
        Ok((&[], &[])) => NodeHash::default(),
        _ => NodeHash::from_slice(rlp),
    }
}
