use std::collections::HashMap;

use crate::error::TrieError;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

use super::db::TrieDB;

/// Libmdbx database representing the trie state
/// It contains a table mapping node hashes to rlp encoded nodes
/// All nodes are stored in the DB and no node is ever removed
use super::{node::Node, node_hash::NodeHash};
pub struct TrieState {
    db: Box<dyn TrieDB>,
    cache: HashMap<NodeHash, Node>,
}

impl TrieState {
    /// Creates a TrieState referring to a db.
    pub fn new(db: Box<dyn TrieDB>) -> TrieState {
        TrieState {
            db,
            cache: Default::default(),
        }
    }

    /// Retrieves a node based on its hash
    pub fn get_node(&self, hash: NodeHash) -> Result<Option<Node>, TrieError> {
        // Decode the node if it is inlined
        if let NodeHash::Inline(encoded) = hash {
            return Ok(Some(Node::decode_raw(&encoded)?));
        }
        if let Some(node) = self.cache.get(&hash) {
            return Ok(Some(node.clone()));
        };
        self.db
            .get(hash.into())?
            .map(|rlp| Node::decode(&rlp).map_err(TrieError::RLPDecode))
            .transpose()
    }

    /// Inserts a node
    pub fn insert_node(&mut self, node: Node, hash: NodeHash) {
        // Don't insert the node if it is already inlined on the parent
        if matches!(hash, NodeHash::Hashed(_)) {
            self.cache.insert(hash, node);
        }
    }

    /// Commits cache changes to DB and clears it
    /// Only writes nodes that follow the root's canonical trie
    pub fn commit(&mut self, root: &NodeHash) -> Result<(), TrieError> {
        self.commit_node(root)?;
        self.cache.clear();
        Ok(())
    }

    // Writes a node and its children into the DB
    fn commit_node(&mut self, node_hash: &NodeHash) -> Result<(), TrieError> {
        let Some(node) = self.cache.remove(node_hash) else {
            // If the node is not in the cache then it means it is already stored in the DB
            return Ok(());
        };
        // Commit children (if any)
        match &node {
            Node::Branch(n) => {
                for child in n.choices.iter() {
                    if child.is_valid() {
                        self.commit_node(child)?;
                    }
                }
            }
            Node::Extension(n) => self.commit_node(&n.child)?,
            Node::Leaf(_) => {}
        }
        // Commit self
        self.db.put(node_hash.into(), node.encode_to_vec())
    }
}
