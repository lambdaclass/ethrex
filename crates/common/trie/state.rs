use std::collections::HashMap;

use crate::error::TrieError;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

use super::db::TrieDB;

/// Database representing the trie state
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

    /// Returns the cache changes that should be committed to the DB
    pub fn get_nodes_to_commit_and_clear_cache(
        &mut self,
        root: &NodeHash,
    ) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut to_commit = vec![];
        self.get_nodes_to_commit_tail_recursive(root, &mut to_commit);
        self.cache.clear();
        to_commit
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
        let mut to_commit = vec![];
        self.get_nodes_to_commit_tail_recursive(node_hash, &mut to_commit);

        self.db.put_batch(to_commit)?;

        Ok(())
    }

    fn get_nodes_to_commit_tail_recursive(
        &mut self,
        node_hash: &NodeHash,
        acc: &mut Vec<(Vec<u8>, Vec<u8>)>,
    ) {
        let Some(node) = self.cache.remove(node_hash) else {
            // If the node is not in the cache then it means it is already stored in the DB
            return;
        };
        // Commit children (if any)
        match &node {
            Node::Branch(n) => {
                for child in n.choices.iter() {
                    if child.is_valid() {
                        self.get_nodes_to_commit_tail_recursive(child, acc);
                    }
                }
            }
            Node::Extension(n) => self.get_nodes_to_commit_tail_recursive(&n.child, acc),
            Node::Leaf(_) => {}
        }
        // Commit self
        acc.push((node_hash.into(), node.encode_to_vec()));
    }

    /// Writes a node directly to the DB bypassing the cache
    pub fn write_node(&mut self, node: Node, hash: NodeHash) -> Result<(), TrieError> {
        // Don't insert the node if it is already inlined on the parent
        if matches!(hash, NodeHash::Hashed(_)) {
            self.db.put(hash.into(), node.encode_to_vec())?;
        }
        Ok(())
    }
}
