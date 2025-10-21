mod branch;
mod extension;
mod leaf;

use std::{
    array,
    sync::{Arc, OnceLock},
};

pub use branch::BranchNode;
use ethrex_rlp::{
    decode::{decode_bytes, decode_rlp_item},
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, OwnedDecoder},
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
    pub fn get_node(&self, db: &dyn TrieDB, path: Nibbles) -> Result<Option<Node>, TrieError> {
        match *self {
            NodeRef::Node(ref node, _) => Ok(Some(node.as_ref().clone())),
            NodeRef::Hash(NodeHash::Inline((data, len))) => {
                let mut inline_node = Vec::from(data);
                inline_node.truncate(len as usize);
                Ok(Some(Node::decode_raw_owned(inline_node.to_vec())?))
            }
            NodeRef::Hash(hash) => db
                .get(path)?
                .filter(|rlp| !rlp.is_empty())
                .and_then(|rlp| match Node::decode_raw_owned(rlp) {
                    Ok(node) => (node.compute_hash() == hash).then_some(Ok(node)),
                    Err(err) => Some(Err(TrieError::RLPDecode(err))),
                })
                .transpose(),
        }
    }

    pub fn is_valid(&self) -> bool {
        match self {
            NodeRef::Node(_, _) => true,
            NodeRef::Hash(hash) => hash.is_valid(),
        }
    }

    pub fn commit(&mut self, path: Nibbles, acc: &mut Vec<(Nibbles, Vec<u8>)>) -> NodeHash {
        match *self {
            NodeRef::Node(ref mut node, ref mut hash) => {
                match Arc::make_mut(node) {
                    Node::Branch(node) => {
                        for (choice, node) in &mut node.choices.iter_mut().enumerate() {
                            node.commit(path.append_new(choice as u8), acc);
                        }
                    }
                    Node::Extension(node) => {
                        node.child.commit(path.concat(&node.prefix), acc);
                    }
                    Node::Leaf(_) => {}
                }
                let hash = *hash.get_or_init(|| node.compute_hash());
                acc.push((path.clone(), node.encode_to_vec()));

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
    pub fn get(&self, db: &dyn TrieDB, path: Nibbles) -> Result<Option<ValueRLP>, TrieError> {
        match self {
            Node::Branch(n) => n.get(db, path),
            Node::Extension(n) => n.get(db, path),
            Node::Leaf(n) => n.get(path),
        }
    }

    /// Inserts a value into the subtrie originating from this node and returns the new root of the subtrie
    pub fn insert(
        self,
        db: &dyn TrieDB,
        path: Nibbles,
        value: impl Into<ValueOrHash>,
    ) -> Result<Node, TrieError> {
        match self {
            Node::Branch(n) => n.insert(db, path, value.into()),
            Node::Extension(n) => n.insert(db, path, value.into()),
            Node::Leaf(n) => n.insert(path, value.into()),
        }
    }

    /// Removes a value from the subtrie originating from this node given its path
    /// Returns the new root of the subtrie (if any) and the removed value if it existed in the subtrie
    pub fn remove(
        self,
        db: &dyn TrieDB,
        path: Nibbles,
    ) -> Result<(Option<Node>, Option<ValueRLP>), TrieError> {
        match self {
            Node::Branch(n) => n.remove(db, path),
            Node::Extension(n) => n.remove(db, path),
            Node::Leaf(n) => n.remove(path),
        }
    }

    /// Traverses own subtrie until reaching the node containing `path`
    /// Appends all encoded nodes traversed to `node_path` (including self)
    /// Only nodes with encoded len over or equal to 32 bytes are included
    pub fn get_path(
        &self,
        db: &dyn TrieDB,
        path: Nibbles,
        node_path: &mut Vec<Vec<u8>>,
    ) -> Result<(), TrieError> {
        match self {
            Node::Branch(n) => n.get_path(db, path, node_path),
            Node::Extension(n) => n.get_path(db, path, node_path),
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

    /// Decodes the node
    pub fn decode_raw_owned(rlp: Vec<u8>) -> Result<Self, RLPDecodeError> {
        let mut decoder = OwnedDecoder::new(rlp)?;
        // Deserialize into node depending on the available fields
        Ok(match decoder.length()? {
            // Leaf or Extension Node
            2 => {
                let compact_path = decoder.decode_next_item()?;
                let path = Nibbles::decode_compact_owned(compact_path);
                if path.is_leaf() {
                    // Decode as Leaf
                    LeafNode {
                        partial: path,
                        value: decoder.decode_next_item()?,
                    }
                    .into()
                } else {
                    // Decode as Extension
                    let child = decoder.get_encoded_item()?;
                    ExtensionNode {
                        prefix: path,
                        child: decode_child_owned(child)?.into(),
                    }
                    .into()
                }
            }
            // Branch Node
            17 => {
                let mut choices = Vec::with_capacity(16);
                for _ in 0..16 {
                    let encoded_child = decoder.get_encoded_item()?;
                    choices.push(decode_child_owned(encoded_child)?.into());
                }
                let choices: [NodeRef; 16] = choices.try_into().unwrap();
                BranchNode {
                    choices,
                    value: decoder.decode_next_item()?,
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

fn decode_child_owned(mut rlp: Vec<u8>) -> Result<NodeHash, RLPDecodeError> {
    let (is_list, payload, rest) = decode_rlp_item(&rlp)?;
    if is_list || !rest.is_empty() {
        return Err(RLPDecodeError::UnexpectedString);
    }

    match payload.len() {
        0 => Ok(NodeHash::default()),
        1..=31 => Ok(NodeHash::from_vec(rlp)),
        32 => {
            let payload_start = payload.as_ptr() as usize - rlp.as_ptr() as usize;
            rlp.drain(..payload_start);
            Ok(NodeHash::from_vec(rlp))
        }
        _ => Err(RLPDecodeError::UnexpectedString),
    }
}
