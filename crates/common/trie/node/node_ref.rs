use crate::Node;
use std::sync::{Arc, OnceLock};

use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use rayon::iter::IntoParallelRefMutIterator;
use rayon::iter::ParallelExtend;
use rayon::iter::ParallelIterator;

use crate::{TrieDB, error::TrieError};

use super::node_hash::NodeHash;

/// A reference to a node.
#[derive(Clone, Debug)]
pub enum NodeRef {
    /// The node is embedded within the reference.
    Node(Arc<Node>, OnceLock<NodeHash>),
    /// The node is in the database, referenced by its hash.
    Hash(NodeHash),
}

impl NodeRef {
    pub fn get_node(&self, db: &dyn TrieDB) -> Result<Option<Node>, TrieError> {
        match *self {
            NodeRef::Node(ref node, _) => Ok(Some(node.as_ref().clone())),
            NodeRef::Hash(NodeHash::Inline((data, len))) => {
                Ok(Some(Node::decode_raw(&data[..len as usize])?))
            }
            NodeRef::Hash(hash @ NodeHash::Hashed(_)) => db
                .get(hash)?
                .map(|rlp| Node::decode(&rlp).map_err(TrieError::RLPDecode))
                .transpose(),
        }
    }

    pub fn is_valid(&self) -> bool {
        match self {
            NodeRef::Node(_, _) => true,
            NodeRef::Hash(hash) => hash.is_valid(),
        }
    }

    /// Returns the hash of the node, computing it if necessary.
    pub fn commit(&mut self, acc: &mut Vec<(NodeHash, Vec<u8>)>) -> NodeHash {
        match *self {
            NodeRef::Node(ref mut node, ref mut hash) => {
                match Arc::make_mut(node) {
                    Node::Branch(node) => {
                        let children_ite = node
                            .choices
                            .par_iter_mut()
                            .map(|node| {
                                let mut acc = Vec::new();
                                node.commit(&mut acc);
                                acc
                            })
                            .collect();
                        for elem in children_iter {
                            acc.push(elem);
                        }
                        //acc.extend(node.choices.par_iter_mut().map(|node| {
                        //    let acc = Vec::new();
                        //    node.commit(&mut acc);
                        //    acc
                        //}));
                    }
                    Node::Extension(node) => {
                        node.child.commit(acc);
                    }
                    Node::Leaf(_) => {}
                }

                let hash = hash.get_or_init(|| node.compute_hash());
                acc.push((*hash, node.encode_to_vec()));

                let hash = *hash;
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
