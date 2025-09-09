use thiserror::Error;

// TODO improve errors
#[derive(Debug, Error)]
pub enum RollupStoreError {
    #[error("DecodeError")]
    DecodeError,
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
    #[error("Database version mismatch. Current: {current}. Expected: {expected}")]
    VersionMismatch { current: u64, expected: u64 },
}
