use ethrex_rlp::error::RLPDecodeError;
use thiserror::Error;

use crate::Nibbles;

#[derive(Debug, Error)]
pub enum TrieError {
    #[error(transparent)]
    RLPDecode(#[from] RLPDecodeError),
    #[error("Verification Error: {0}")]
    Verify(String),
    #[error("Inconsistent internal tree structure")]
    InconsistentTree,
    #[error("Lock Error: Panicked when trying to acquire a lock")]
    LockError,
    #[error("Database error: {0}")]
    DbError(anyhow::Error),
    #[error("Invalid trie input")]
    InvalidInput,

    // TODO: make these a sub-variant of InconsistentTree ?
    #[error("Inconsistent internal tree structure: root not found")]
    RootNotFound,
    #[error("Inconsistent internal tree structure: found extension in place of leaf {0:?}")]
    FoundExtensionInPlaceOfLeaf(Nibbles),
    #[error("Inconsistent internal tree structure: found branch in place of leaf {0:?}")]
    FoundBranchInPlaceOfLeaf(Nibbles),
    #[error("Inconsistent internal tree structure: leaf not found {0:?}")]
    LeafNotFound(Nibbles),
}
