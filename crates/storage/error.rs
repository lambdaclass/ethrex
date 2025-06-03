use ethrex_rlp::error::RLPDecodeError;
use ethrex_trie::TrieError;
use redb::{CommitError, DatabaseError, StorageError, TableError, TransactionError};
use thiserror::Error;

// TODO improve errors
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("DecodeError")]
    DecodeError,
    #[error("Libmdbx error: {0}")]
    LibmdbxError(anyhow::Error),
    #[error("Redb Storage error: {0}")]
    RedbStorageError(#[from] StorageError),
    #[error("Redb Table error: {0}")]
    RedbTableError(#[from] TableError),
    #[error("Redb Commit error: {0}")]
    RedbCommitError(#[from] CommitError),
    #[error("Redb Transaction error: {0}")]
    RedbTransactionError(#[from] TransactionError),
    #[error("Redb Database error: {0}")]
    RedbDatabaseError(#[from] DatabaseError),
    #[error("Redb Cast error")]
    RedbCastError,
    #[error("{0}")]
    Custom(String),
    #[error(transparent)]
    RLPDecode(#[from] RLPDecodeError),
    #[error(transparent)]
    Trie(#[from] TrieError),
    #[error("missing store: is an execution DB being used instead?")]
    MissingStore,
    #[error("Could not open DB for reading")]
    ReadError,
    #[error("Could not instantiate cursor for table {0}")]
    CursorError(String),
    #[error("Missing latest block number")]
    MissingLatestBlockNumber,
    #[error("Missing earliest block number")]
    MissingEarliestBlockNumber,
    #[error("Failed to lock mempool for writing")]
    MempoolWriteLock(String),
    #[error("Failed to lock mempool for reading")]
    MempoolReadLock(String),
}
