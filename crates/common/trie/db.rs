use crate::{Nibbles, Node, NodeHandle, error::TrieError};

pub trait TrieDB: Send + Sync {
    fn get(&self, key: NodeHandle) -> Result<Option<Node>, TrieError>;
    fn get_path(&self, path: Nibbles) -> Result<Option<Node>, TrieError>;
}
