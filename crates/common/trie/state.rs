use std::collections::HashMap;

use crate::error::TrieError;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

use super::db::TrieDB;

/// Database representing the trie state
/// It contains a table mapping node hashes to rlp encoded nodes
/// All nodes are stored in the DB and no node is ever removed
use super::{node::Node, node_hash::NodeHash};

// Struct that wraps around a mutable
// hashmap to be used as a cache for the Trie.
//
// I'not a fan of get/set methods, but the alternative
// is to make the TrieState struct mutable in a lot of places
// where not needed, potentially exposing the implementation
// to bugs. Thus, the alternative is to use a RefCell.
// Since RefCells can panic at runtime, I find it safer to
// use get/set methods to properly drop the borrow of
// the RefCell after accessing or modifying the map.
//
// Furthermore, if we ever want to have an eviction
// policy, this struct can be useful for it.
struct TrieStateCache {
    inner: std::cell::RefCell<HashMap<NodeHash, Node>>,
}

impl TrieStateCache {
    pub fn new_empty() -> Self {
        Self {
            inner: Default::default(),
        }
    }
    pub fn insert(&self, key: NodeHash, value: Node) {
        self.inner.borrow_mut().insert(key, value);
    }
    pub fn get(&self, key: &NodeHash) -> Option<Node> {
        self.inner.borrow().get(key).cloned()
    }
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }
    pub fn remove(&self, key: &NodeHash) -> Option<Node> {
        self.inner.borrow_mut().remove(key)
    }
}

pub struct TrieState {
    db: Box<dyn TrieDB>,
    cache: TrieStateCache,
}

impl TrieState {
    /// Creates a TrieState referring to a db.
    pub fn new(db: Box<dyn TrieDB>) -> TrieState {
        TrieState {
            db,
            cache: TrieStateCache::new_empty(),
        }
    }

    /// Retrieves a node based on its hash
    pub fn get_node(&self, hash: NodeHash) -> Result<Option<Node>, TrieError> {
        // Decode the node if it is inlined
        if let NodeHash::Inline(_) = hash {
            return Ok(Some(Node::decode_raw(hash.as_ref())?));
        }
        match self.cache.get(&hash) {
            Some(node) => Ok(Some(node.clone())),
            None => {
                let Some(db_result) = self
                    .db
                    .get(hash)?
                    .map(|rlp| Node::decode(&rlp).map_err(TrieError::RLPDecode))
                    .transpose()?
                else {
                    return Ok(None);
                };
                self.cache.insert(hash, db_result.clone());
                Ok(Some(db_result))
            }
        }
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
        let mut to_commit = vec![];
        self.commit_node_tail_recursive(node_hash, &mut to_commit)?;

        self.db.put_batch(to_commit)?;

        Ok(())
    }

    // Writes a node and its children into the DB
    fn commit_node_tail_recursive(
        &mut self,
        node_hash: &NodeHash,
        acc: &mut Vec<(NodeHash, Vec<u8>)>,
    ) -> Result<(), TrieError> {
        let Some(node) = self.cache.remove(node_hash) else {
            // If the node is not in the cache then it means it is already stored in the DB
            return Ok(());
        };
        // Commit children (if any)
        match &node {
            Node::Branch(n) => {
                for child in n.choices.iter() {
                    if child.is_valid() {
                        self.commit_node_tail_recursive(child, acc)?;
                    }
                }
            }
            Node::Extension(n) => self.commit_node_tail_recursive(&n.child, acc)?,
            Node::Leaf(_) => {}
        }
        // Commit self
        acc.push((*node_hash, node.encode_to_vec()));

        Ok(())
    }

    /// Writes a node directly to the DB bypassing the cache
    pub fn write_node(&mut self, node: Node, hash: NodeHash) -> Result<(), TrieError> {
        // Don't insert the node if it is already inlined on the parent
        if matches!(hash, NodeHash::Hashed(_)) {
            self.db.put(hash, node.encode_to_vec())?;
        }
        Ok(())
    }

    /// Writes a node batch directly to the DB bypassing the cache
    pub fn write_node_batch(&mut self, nodes: &[Node]) -> Result<(), TrieError> {
        // Don't insert the node if it is already inlined on the parent
        let key_values = nodes
            .iter()
            .filter_map(|node| {
                let hash = node.compute_hash();
                matches!(hash, NodeHash::Hashed(_)).then(|| (hash, node.encode_to_vec()))
            })
            .collect();
        self.db.put_batch(key_values)?;
        Ok(())
    }
}
