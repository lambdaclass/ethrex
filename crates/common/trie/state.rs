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
    pub fn get_node(&mut self, hash: NodeHash) -> Result<Option<Node>, TrieError> {
        // Decode the node if it is inlined
        if let NodeHash::Inline(_) = hash {
            return Ok(Some(Node::decode_raw(hash.as_ref())?));
        }
        match self.cache.get(&hash) {
            None => {
                let node = self
                    .db
                    .get(hash)?
                    .map(|rlp| Node::decode(&rlp).map_err(TrieError::RLPDecode))
                    .transpose()?;
                // FIXME: Change this to Option<Node>
                self.cache.insert(hash, node.clone().unwrap());
                return Ok(node);
            }
            Some(cached_node) => return Ok(Some(cached_node.clone())),
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
