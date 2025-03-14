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
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, TrieError> {
        let mut to_commit = vec![];
        self.commit_node_tail_recursive(root, &mut to_commit)?;
        self.cache.clear();
        Ok(to_commit)
    }

    /// Commits cache changes to DB and clears it
    /// Only writes nodes that follow the root's canonical trie
    pub fn commit(&mut self, root: NodeHash) -> Result<(), TrieError> {
        self.commit_node(root)?;
        self.cache.clear();
        Ok(())
    }

    // Writes a node and its children into the DB
    fn commit_node(&mut self, node_hash: NodeHash) -> Result<(), TrieError> {
        let mut to_commit = vec![];
        let mut stack = vec![node_hash];

        while let Some(current_hash) = stack.pop() {
            let Some(node) = self.cache.remove(&current_hash) else {
                continue;
            };

            let encoded_node = node.encode_to_vec();
            match node {
                Node::Branch(n) => {
                    for child in n.choices.into_iter() {
                        if child.is_valid() {
                            stack.push(child);
                        }
                    }
                }
                Node::Extension(n) => {
                    stack.push(n.child);
                }
                Node::Leaf(_) => {}
            }

            to_commit.push((current_hash.into(), encoded_node));
        }

        self.db.put_batch(to_commit)?;

        Ok(())
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
