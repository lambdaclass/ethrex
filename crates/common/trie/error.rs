use ethereum_types::H256;
use ethrex_rlp::error::RLPDecodeError;
use thiserror::Error;

use crate::Nibbles;

#[derive(Debug, Error)]
pub enum TrieError {
    #[error(transparent)]
    RLPDecode(#[from] RLPDecodeError),
    #[error("Verification Error: {0}")]
    Verify(String),
    #[error("Inconsistent internal tree structure: {0}")]
    InconsistentTree(#[from] InconsistentTreeError),
    #[error("Lock Error: Panicked when trying to acquire a lock")]
    LockError,
    #[error("Database error: {0}")]
    DbError(anyhow::Error),
    #[error("Invalid trie input")]
    InvalidInput,
}

#[derive(Debug, Error)]
pub enum InconsistentTreeError {
    #[error("Failed to insert node as child of Extention node: {0}")]
    ExtensionNodeInsertionError(Box<ExtensionNodeErrorData>),
    #[error("Node with hash {0:#x} not found in Branch Node with hash {1:#x} using path {2:?}")]
    NodeNotFoundOnBranchNode(H256, H256, Nibbles),
    #[error("{0}")]
    NodeNotFoundOnExtensionNode(Box<ExtensionNodeErrorData>),
    #[error("Root node with hash {0:#x} not found")]
    RootNotFound(H256),
}

#[derive(Debug)]
pub struct ExtensionNodeErrorData {
    pub node_hash: H256,
    pub extension_node_hash: H256,
    pub extension_node_prefix: Nibbles,
    pub node_path: Nibbles,
}

impl std::fmt::Display for ExtensionNodeErrorData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Node with hash {:#x} not found as child of Extension Node with hash {:#x} and prefix {:?} using path {:?}",
            self.node_hash, self.extension_node_hash, self.extension_node_hash, self.node_path
        )
    }
}
