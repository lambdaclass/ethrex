use ethereum_types::H256;
use ethrex_rlp::error::RLPDecodeError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TrieError {
    #[error(transparent)]
    RLPDecode(#[from] RLPDecodeError),
    #[error("Verification Error: {0}")]
    Verify(String),
    #[error("Inconsistent internal tree structure: Node with hash {0:?} not found")]
    InconsistentTree(H256),
    #[error(
        "Inconsistent internal tree structure: Node with hash {0:?} not found from Intermediate Node with hash {1:?}"
    )]
    IntermediateNodeNotFound(H256, H256),
    #[error("Root node with hash {0:#x} not found")]
    RootNotFound(H256),
    #[error("Lock Error: Panicked when trying to acquire a lock")]
    LockError,
    #[error("Database error: {0}")]
    DbError(anyhow::Error),
    #[error("Invalid trie input")]
    InvalidInput,
}
