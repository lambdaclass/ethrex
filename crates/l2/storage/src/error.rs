#[cfg(feature = "redb")]
use redb::{CommitError, DatabaseError, StorageError, TableError, TransactionError};
use thiserror::Error;

// TODO improve errors
#[derive(Debug, Error)]
pub enum RollupStoreError {
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
    RedbTransactionError(#[from] Box<TransactionError>),
    #[error("Redb Database error: {0}")]
    #[cfg(feature = "redb")]
    RedbDatabaseError(#[from] DatabaseError),
    #[error("Redb Cast error")]
    #[cfg(feature = "redb")]
    RedbCastError,
    #[cfg(feature = "sql")]
    #[error("Limbo Query error: {0}")]
    SQLQueryError(#[from] libsql::Error),
    #[cfg(feature = "sql")]
    #[error("SQL Query error: unexpected type found while querying DB")]
    SQLInvalidTypeError,
    #[error("{0}")]
    Custom(String),
    #[error("Bincode (de)serialization error: {0}")]
    BincodeError(#[from] bincode::Error),
}
