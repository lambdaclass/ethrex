mod branch;
mod extension;
mod leaf;

use std::{
    array,
    sync::{Arc, OnceLock},
};

pub use branch::BranchNode;
use ethrex_rlp::{
    decode::{RLPDecode, decode_bytes, decode_rlp_item},
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
    pub fn get_node(&self, db: &dyn TrieDB, path: Nibbles) -> Result<Option<Arc<Node>>, TrieError> {
        match self {
            NodeRef::Node(node, _) => Ok(Some(node.clone())),
            NodeRef::Hash(NodeHash::Inline((data, len))) => {
                let mut inline_node = Vec::from(data);
                inline_node.truncate(*len as usize);
                Ok(Some(Arc::new(Node::decode_raw_owned(inline_node.to_vec())?)))
            }
            NodeRef::Hash(hash @ NodeHash::Hashed(_)) => db
                .get(path)?
                .filter(|rlp| !rlp.is_empty())
                .and_then(|rlp| match Node::decode(&rlp) {
                    Ok(node) => (node.compute_hash() == *hash).then_some(Ok(Arc::new(node))),
                    Err(err) => Some(Err(TrieError::RLPDecode(err))),
                })
                .transpose(),
        }
    }

    pub fn get_node_mut(
        &mut self,
        db: &dyn TrieDB,
        path: Nibbles,
    ) -> Result<Option<&mut Node>, TrieError> {
        match self {
            NodeRef::Node(node, _) => Ok(Some(Arc::make_mut(node))),
            NodeRef::Hash(hash @ NodeHash::Inline(_)) => {
                let node = Node::decode_raw(hash.as_ref())?;
                *self = NodeRef::Node(Arc::new(node), OnceLock::from(*hash));
                self.get_node_mut(db, path)
            }
            NodeRef::Hash(hash @ NodeHash::Hashed(_)) => {
                let Some(node) = db
                    .get(path.clone())?
                    .filter(|rlp| !rlp.is_empty())
                    .and_then(|rlp| match Node::decode(&rlp) {
                        Ok(node) => (node.compute_hash() == *hash).then_some(Ok(node)),
                        Err(err) => Some(Err(TrieError::RLPDecode(err))),
                    })
                    .transpose()?
                else {
                    return Ok(None);
                };
                *self = NodeRef::Node(Arc::new(node), OnceLock::from(*hash));
                self.get_node_mut(db, path)
            }
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
        *self.compute_hash_ref()
    }

    pub fn compute_hash_ref(&self) -> &NodeHash {
        match self {
            NodeRef::Node(node, hash) => hash.get_or_init(|| node.compute_hash()),
            NodeRef::Hash(hash) => hash,
        }
    }

    pub fn memoize_hashes(&self) {
        if let NodeRef::Node(node, hash) = &self {
            if hash.get().is_none() {
                node.memoize_hashes();
                let _ = hash.set(node.compute_hash());
            }
        }
    }

    pub fn clear_hash(&mut self) {
        if let NodeRef::Node(_, hash) = self {
            hash.take();
        }
    }

    /// # SAFETY: caller must ensure the hash is correct for the node.
    /// Otherwise, the `Trie` will silently produce incorrect results and may
    /// fail to query other nodes from the `TrieDB`.
    pub fn with_hash_unchecked(self, hash: NodeHash) -> Self {
        if let NodeRef::Node(_, node_hash) = &self {
            let _ = node_hash.set(hash);
        }
        self
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

    /// Inserts a value into the subtrie originating from this node.
    pub fn insert(
        &mut self,
        db: &dyn TrieDB,
        path: Nibbles,
        value: impl Into<ValueOrHash>,
    ) -> Result<(), TrieError> {
        let new_node = match self {
            Node::Branch(n) => {
                n.insert(db, path, value.into())?;
                Ok(None)
            }
            Node::Extension(n) => n.insert(db, path, value.into()),
            Node::Leaf(n) => n.insert(path, value.into()),
        };
        if let Some(new_node) = new_node? {
            *self = new_node;
        }
        Ok(())
    }

    /// Removes a value from the subtrie originating from this node given its path
    /// Returns a bool indicating if the new subtrie is empty, and the removed value if it existed in the subtrie
    pub fn remove(
        &mut self,
        db: &dyn TrieDB,
        path: Nibbles,
    ) -> Result<(bool, Option<ValueRLP>), TrieError> {
        let (new_root, value) = match self {
            Node::Branch(n) => n.remove(db, path),
            Node::Extension(n) => n.remove(db, path),
            Node::Leaf(n) => n.remove(path),
        }?;
        match new_root {
            Some(NodeRemoveResult::New(new_root)) => {
                *self = new_root;
                Ok((false, value))
            }
            Some(NodeRemoveResult::Mutated) => Ok((false, value)),
            None => Ok((true, value)),
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
        self.memoize_hashes();
        match self {
            Node::Branch(n) => n.compute_hash(),
            Node::Extension(n) => n.compute_hash(),
            Node::Leaf(n) => n.compute_hash(),
        }
    }

    /// Recursively memoizes the hashes of all nodes of the subtrie that has
    /// `self` as root (post-order traversal)
    pub fn memoize_hashes(&self) {
        match self {
            Node::Branch(n) => {
                for child in &n.choices {
                    child.memoize_hashes();
                }
            }
            Node::Extension(n) => n.child.memoize_hashes(),
            _ => {}
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

/// Used as return type for `Node` remove operations that may resolve into either:
/// - a mutation of the `Node`
/// - a new `Node`
pub enum NodeRemoveResult {
    Mutated,
    New(Node),
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
