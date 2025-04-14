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
    next_unhashed: usize,
}

impl TrieState {
    /// Creates a TrieState referring to a db.
    pub fn new(db: Box<dyn TrieDB>) -> TrieState {
        TrieState {
            db,
            cache: Default::default(),
            next_unhashed: 0,
        }
    }

    pub fn alloc_unhashed(&mut self) -> NodeHash {
        let hash = NodeHash::UnhashedIndex(self.next_unhashed);
        self.next_unhashed += 1;
        hash
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
        if matches!(hash, NodeHash::Hashed(_) | NodeHash::UnhashedIndex(_)) {
            self.cache.insert(hash, node);
        }
    }

    /// Commits cache changes to DB and clears it
    /// Only writes nodes that follow the root's canonical trie
    pub fn commit(&mut self, root: &NodeHash) -> Result<Option<NodeHash>, TrieError> {
        let root_hash = self.commit_node(root)?;
        self.cache.clear();
        Ok(root_hash)
    }

    // Writes a node and its children into the DB
    fn commit_node(&mut self, node_hash: &NodeHash) -> Result<Option<NodeHash>, TrieError> {
        let mut to_commit = vec![];
        let hash = self.commit_node_tail_recursive(node_hash.clone(), &mut to_commit)?;
        if hash.is_some() {
            self.db.put_batch(to_commit)?;
        }

        Ok(hash)
    }

    // Writes a node and its children into the DB
    fn commit_node_tail_recursive(
        &mut self,
        mut node_hash: NodeHash,
        acc: &mut Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<Option<NodeHash>, TrieError> {
        let Some(mut node) = self.cache.remove(&node_hash) else {
            // If the node is not in the cache then it means it is already stored in the DB
            return Ok(None);
        };
        let mut dirty = matches!(node_hash, NodeHash::UnhashedIndex(_));
        // Commit children (if any)
        match &mut node {
            Node::Branch(n) => {
                for child in n.choices.iter_mut() {
                    if child.is_valid() {
                        let child_hash = self.commit_node_tail_recursive(child.clone(), acc)?;
                        if let Some(hash) = child_hash {
                            dirty = true;
                            *child = hash;
                        }
                    }
                }
            }
            Node::Extension(n) => {
                let child_hash = self.commit_node_tail_recursive(n.child.clone(), acc)?;
                if let Some(hash) = child_hash {
                    dirty = true;
                    n.child = hash;
                }
            }
            Node::Leaf(_) => {}
        }
        // Commit self
        if dirty {
            node_hash = node.compute_hash();
        }
        acc.push((node_hash.clone().into(), node.encode_to_vec()));

        Ok(dirty.then_some(node_hash.clone()))
    }

    /// Writes a node directly to the DB bypassing the cache
    pub fn write_node(&mut self, node: Node, hash: NodeHash) -> Result<(), TrieError> {
        // Don't insert the node if it is already inlined on the parent
        if matches!(hash, NodeHash::Hashed(_)) {
            self.db.put(hash.into(), node.encode_to_vec())?;
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
                matches!(hash, NodeHash::Hashed(_)).then(|| (hash.into(), node.encode_to_vec()))
            })
            .collect();
        self.db.put_batch(key_values)?;
        Ok(())
    }
}
