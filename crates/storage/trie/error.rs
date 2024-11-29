use ethrex_rlp::error::RLPDecodeError;
#[cfg(feature = "redb")]
use redb::{CommitError, StorageError, TableError, TransactionError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TrieError {
    #[cfg(feature = "libmdbx")]
    #[error("Libmdbx error: {0}")]
    LibmdbxError(anyhow::Error),
    #[cfg(feature = "redb")]
    #[error("Redb Storage error: {0}")]
    RedbStorageError(#[from] StorageError),
    #[cfg(feature = "redb")]
    #[error("Redb Table error: {0}")]
    #[cfg(feature = "redb")]
    RedbTableError(#[from] TableError),
    #[error("Redb Commit error: {0}")]
    #[cfg(feature = "redb")]
    RedbCommitError(#[from] CommitError),
    #[error("Redb Transaction error: {0}")]
    #[cfg(feature = "redb")]
    RedbTransactionError(#[from] TransactionError),
    #[error(transparent)]
    RLPDecode(#[from] RLPDecodeError),
    #[error("Verification Error: {0}")]
    Verify(String),
    #[error("Inconsistent internal tree structure")]
    InconsistentTree,
}
