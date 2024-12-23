use ethrex_rlp::error::RLPDecodeError;
use ethrex_trie::TrieError;
#[cfg(feature = "redb")]
use redb::{CommitError, DatabaseError, StorageError, TableError, TransactionError};
use thiserror::Error;

// TODO improve errors
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("DecodeError")]
    DecodeError,
    #[cfg(feature = "libmdbx")]
    #[error("Libmdbx error: {0}")]
    LibmdbxError(anyhow::Error),
    #[cfg(feature = "redb")]
    #[error("Redb Storage error: {0}")]
    RedbStorageError(#[from] StorageError),
    #[cfg(feature = "redb")]
    #[error("Redb Table error: {0}")]
    RedbTableError(#[from] TableError),
    #[cfg(feature = "redb")]
    #[error("Redb Commit error: {0}")]
    RedbCommitError(#[from] CommitError),
    #[cfg(feature = "redb")]
    #[error("Redb Transaction error: {0}")]
    RedbTransactionError(#[from] TransactionError),
    #[error("Redb Database error: {0}")]
    #[cfg(feature = "redb")]
    RedbDatabaseError(#[from] DatabaseError),
    #[error("Redb Cast error")]
    #[cfg(feature = "redb")]
    RedbCastError,
    #[error("{0}")]
    Custom(String),
    #[error(transparent)]
    RLPDecode(#[from] RLPDecodeError),
    #[error(transparent)]
    Trie(#[from] TrieError),
    #[error("missing store: is an execution DB being used instead?")]
    MissingStore,
    #[error("Missing latest block number")]
    MissingLatestBlockNumber,
    #[error("Missing earliest block number")]
    MissingEarliestBlockNumber,
}
