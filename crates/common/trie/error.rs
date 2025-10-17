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
    #[error(
        "Branch Node with hash {0:#x} not found to insert as child of Extension Node with hash {1:#x} and prefix {2:?} using path {3:?}"
    )]
    ExtensionNodeInsertionError(H256, H256, Nibbles, Nibbles),
    #[error("Node with hash {0:#x} not found in Branch Node with hash {1:#x} using path {2:?}")]
    NodeNotFoundOnBranchNode(H256, H256, Nibbles),
    #[error(
        "Node with hash {0:#x} not found as child of Extension Node with hash {1:#x} and prefix {2:?} using path {3:?}"
    )]
    NodeNotFoundOnExtensionNode(H256, H256, Nibbles, Nibbles),
    #[error("Root node with hash {0:#x} not found")]
    RootNotFound(H256),
}
