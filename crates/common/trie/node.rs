mod branch;
mod extension;
mod leaf;

use std::sync::{Arc, OnceLock};

pub use branch::BranchNode;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
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
                Ok(Some(Node::decode(&data[..len as usize])?))
            }
            NodeRef::Hash(hash) => db
                .get(path)?
                .filter(|rlp| !rlp.is_empty())
                .and_then(|rlp| match Node::decode(&rlp) {
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

    pub fn commit(&mut self, path: Nibbles, acc: &mut Vec<(Nibbles, Vec<u8>)>, buf: &mut Vec<u8>) -> NodeHash {
        match *self {
            NodeRef::Node(ref mut node, ref mut hash) => {
                match Arc::make_mut(node) {
                    Node::Branch(node) => {
                        for (choice, node) in &mut node.choices.iter_mut().enumerate() {
                            node.commit(path.append_new(choice as u8), acc, buf);
                        }
                    }
                    Node::Extension(node) => {
                        node.child.commit(path.concat(&node.prefix), acc, buf);
                    }
                    Node::Leaf(_) => {}
                }
                node.encode(buf);
                let hash = *hash.get_or_init(|| NodeHash::from_encoded_raw(buf));
                acc.push((path.clone(), buf.to_vec()));
                if let Node::Leaf(leaf) = node.as_ref() {
                    acc.push((path.concat(&leaf.partial), leaf.value.clone()));
                }
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

    /// Computes the node's hash
    pub fn compute_hash(&self) -> NodeHash {
        match self {
            Node::Branch(n) => n.compute_hash(),
            Node::Extension(n) => n.compute_hash(),
            Node::Leaf(n) => n.compute_hash(),
        }
    }
}
