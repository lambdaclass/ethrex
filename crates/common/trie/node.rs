mod branch;
mod extension;
mod leaf;

use std::{array, sync::Arc};

pub use branch::BranchNode;
use ethrex_rlp::{
    decode::{RLPDecode, decode_bytes},
    error::RLPDecodeError,
    structs::Decoder,
};
pub use extension::ExtensionNode;
pub use leaf::LeafNode;
use tracing::info;

use crate::{EMPTY_TRIE_HASH, TrieDB, error::TrieError, nibbles::Nibbles};

use super::{ValueRLP, node_hash::NodeHash};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeHandle(pub u64);
#[derive(Clone, Copy, Debug)]
pub struct CodeHandle(pub u64);

// FIXME: can't use hash invalidation to mark dirtiness due to inline nodes,
// inserting an inline node and then overwriting the hash that contains it
// would break. Just use the high bit of the handle instead.
/// A reference to a node.
#[derive(Clone, Debug)]
pub struct NodeRef {
    pub value: Option<Arc<Node>>,
    pub hash: NodeHash,
    pub handle: NodeHandle,
}

impl NodeRef {
    pub fn is_dirty(&self) -> bool {
        let NodeHandle(bits) = self.handle;
        (bits & (1 << 63)) != 0
    }

    pub fn set_dirty(&mut self) {
        let NodeHandle(bits) = self.handle;
        self.handle = NodeHandle(bits | (1 << 63));
    }

    pub fn clear_dirty(&mut self) {
        let NodeHandle(bits) = self.handle;
        self.handle = NodeHandle(bits & !(1 << 63));
    }

    pub fn get_node(&self, db: &dyn TrieDB) -> Result<Option<Node>, TrieError> {
        if !self.hash.is_valid() && self.value.is_none() {
            // info!("!VALID, RETURN NONE");
            return Ok(None);
        }
        if let Some(ref node) = self.value {
            // info!("GOT NODE");
            return Ok(Some(node.as_ref().clone()));
        };
        if let NodeHash::Inline((rlp, len)) = self.hash {
            // info!("GOT INLINE");
            return <Node as RLPDecode>::decode(&rlp[..len as usize])
                .map_err(TrieError::RLPDecode)
                .map(Some);
        }
        // info!("GET FROM DB");
        db.get(self.handle)
    }

    pub fn is_valid(&self) -> bool {
        match self.value {
            Some(_) => true,
            None => self.hash.is_valid(),
        }
    }

    pub fn commit(&mut self, acc: &mut Vec<NodeRef>) -> NodeHash {
        if !self.is_dirty() && self.hash.is_valid() {
            // info!(hash = hex::encode(self.hash.finalize()), "NOT DIRTY");
            return self.hash;
        }
        let Some(node) = &mut self.value else {
            // Dirty but no node
            // info!("NO NODE");
            return NodeHash::Inline(([0u8; 31], 0));
        };
        match Arc::make_mut(node) {
            Node::Branch(node) => {
                // info!("BRANCH");
                for node in &mut node.choices {
                    node.commit(acc);
                }
            }
            Node::Extension(node) => {
                // info!("EXTENSION");
                // If this extension comes from splitting an older one
                // the child might actually be clean.
                node.child.commit(acc);
            }
            Node::Leaf(_) => {}
        }

        // Since the node was dirty, the hash is guaranteed to be stale.
        let hash = node.compute_hash();
        let temporary_handle = (1u64 << 63) + acc.len() as u64;
        self.handle = NodeHandle(temporary_handle);
        self.hash = hash;

        acc.push(self.clone());

        // Node is committed, clear dirty flag.
        // FIXME: actually I should just mark the children here, otherwise
        // when we clone all children self-marked clean, breaking serialization.
        // self.clear_dirty();

        hash
    }

    pub fn compute_hash(&self) -> NodeHash {
        // Three cases:
        // 1. Hash already computed and node unmodified since (self.hash.is_valid());
        // 2. Absent node with empty hash => no node;
        // 3. Node exists but hash is empty => signals dirty, compute.
        let current_hash = self.hash;
        if current_hash.is_valid() {
            // info!(
            //     hash = hex::encode(self.hash.finalize()),
            //     status = "VALID HASH",
            //     "COMPUTE HASH"
            // );
            return current_hash;
        }
        let Some(ref node) = self.value else {
            // info!(status = "NO VALUE", "COMPUTE HASH");
            return NodeHash::Inline(([0u8; 31], 0));
        };
        // FIXME: should update the cache but then I have to deal with mutex
        // or use OnceLock
        // info!(status = "COMPUTE", "COMPUTE HASH");
        node.compute_hash()
    }
}

impl Default for NodeRef {
    fn default() -> Self {
        Self {
            hash: NodeHash::default(),
            value: None,
            handle: NodeHandle(1 << 63), // New nodes are always marked dirty
        }
    }
}

impl From<Node> for NodeRef {
    fn from(value: Node) -> Self {
        Self {
            hash: NodeHash::default(),
            value: Some(Arc::new(value)),
            handle: NodeHandle(1 << 63), // New nodes are always marked dirty
        }
    }
}

impl From<NodeHash> for NodeRef {
    fn from(value: NodeHash) -> Self {
        Self {
            hash: value,
            value: None,
            handle: NodeHandle(1 << 63), // New nodes are always marked dirty
        }
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

    pub fn insert(
        self,
        db: &dyn TrieDB,
        path: Nibbles,
        value: impl Into<ValueOrHash>,
    ) -> Result<Node, TrieError> {
        match self {
            Node::Branch(n) => n.insert(db, path, value.into(), None),
            Node::Extension(n) => n.insert(db, path, value.into(), None),
            Node::Leaf(n) => n.insert(path, value.into(), None),
        }
    }

    /// Inserts a value into the subtrie originating from this node and returns the new root of the subtrie
    pub fn insert_with_link(
        self,
        db: &dyn TrieDB,
        path: Nibbles,
        value: impl Into<ValueOrHash>,
        link: Option<NodeHandle>,
    ) -> Result<Node, TrieError> {
        match self {
            Node::Branch(n) => n.insert(db, path, value.into(), link),
            Node::Extension(n) => n.insert(db, path, value.into(), link),
            Node::Leaf(n) => n.insert(path, value.into(), link),
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
                        link: None,
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
