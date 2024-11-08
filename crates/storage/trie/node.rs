mod branch;
mod extension;
mod leaf;

pub use branch::BranchNode;
pub use extension::ExtensionNode;
pub use leaf::LeafNode;

use crate::{error::TrieError, nibbles::Nibbles};

use super::{node_hash::NodeHash, state::TrieState, ValueRLP};

/// A Node in an Ethereum Compatible Patricia Merkle Trie
#[derive(Debug, Clone)]
pub enum Node {
    Branch(BranchNode),
    Extension(ExtensionNode),
    Leaf(LeafNode),
}

impl From<BranchNode> for Node {
    fn from(val: BranchNode) -> Self {
        Node::Branch(val)
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
    pub fn get(&self, state: &TrieState, path: Nibbles) -> Result<Option<ValueRLP>, TrieError> {
        match self {
            Node::Branch(n) => n.get(state, path),
            Node::Extension(n) => n.get(state, path),
            Node::Leaf(n) => n.get(path),
        }
    }

    /// Inserts a value into the subtrie originating from this node and returns the new root of the subtrie
    pub fn insert(
        self,
        state: &mut TrieState,
        path: Nibbles,
        value: ValueRLP,
    ) -> Result<Node, TrieError> {
        match self {
            Node::Branch(n) => n.insert(state, path, value),
            Node::Extension(n) => n.insert(state, path, value),
            Node::Leaf(n) => n.insert(state, path, value),
        }
    }

    /// Removes a value from the subtrie originating from this node given its path
    /// Returns the new root of the subtrie (if any) and the removed value if it existed in the subtrie
    pub fn remove(
        self,
        state: &mut TrieState,
        path: Nibbles,
    ) -> Result<(Option<Node>, Option<ValueRLP>), TrieError> {
        match self {
            Node::Branch(n) => n.remove(state, path),
            Node::Extension(n) => n.remove(state, path),
            Node::Leaf(n) => n.remove(path),
        }
    }

    /// Traverses own subtrie until reaching the node containing `path`
    /// Appends all encoded nodes traversed to `node_path` (including self)
    /// Only nodes with encoded len over or equal to 32 bytes are included
    pub fn get_path(
        &self,
        state: &TrieState,
        path: Nibbles,
        node_path: &mut Vec<Vec<u8>>,
    ) -> Result<(), TrieError> {
        match self {
            Node::Branch(n) => n.get_path(state, path, node_path),
            Node::Extension(n) => n.get_path(state, path, node_path),
            Node::Leaf(n) => n.get_path(node_path),
        }
    }

    pub fn insert_self(self, state: &mut TrieState) -> Result<NodeHash, TrieError> {
        match self {
            Node::Branch(n) => n.insert_self(state),
            Node::Extension(n) => n.insert_self(state),
            Node::Leaf(n) => n.insert_self(state),
        }
    }

    pub fn encode_raw(self) -> Vec<u8> {
        match self {
            Node::Branch(n) => n.encode_raw(),
            Node::Extension(n) => n.encode_raw(),
            Node::Leaf(n) => n.encode_raw(),
        }
    }
}
