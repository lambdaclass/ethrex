use ethrex_common::types::BlockNumber;
use ethrex_storage::error::StoreError;
use std::io;
// use alloy_primitives::BlockNumber;
// use reth_storage_errors::{db::DatabaseError, provider::ProviderError};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Directory not readable: {0}, {1}")]
    DirNotReadable(String, io::Error),
    #[error("Directory not writable: {0}, {1}")]
    DirNotWritable(String, io::Error),
    #[error("Database could not be initialised: {0}")]
    DatabaseInitialisationError(String),
    // #[error(transparent)]
    // ProviderError(#[from] ProviderError),
    #[error("Read transaction error: {0}")]
    ReadTransactionError(#[from] StoreError),
    #[error("{0}")]
    Other(String),
    #[error("State root mismatch: {0}")]
    StateRootError(BlockNumber),
    // #[error("Reth state root error: {0}")]
    // RethStateRootError(#[from] reth_execution_errors::StateRootError),
    // #[error("Parallel state root error: {0}")]
    // ParallelStateRootError(#[from] reth_trie_parallel::root::ParallelStateRootError),
    #[error("Block not found: {0}")]
    BlockNotFound(BlockNumber),
}

// impl From<Error> for ProviderError {
//     fn from(value: Error) -> Self {
//         match value {
//             Error::DirNotReadable(path, _) => ProviderError::FsPathError(path),
//             Error::DirNotWritable(path, _) => ProviderError::FsPathError(path),
//             Error::DatabaseInitialisationError(e) => {
//                 ProviderError::Database(DatabaseError::Other(e))
//             }
//             Error::ProviderError(e) => e,
//             Error::ReadTransactionError(e) => ProviderError::Database(e),
//             Error::Other(e) => ProviderError::Database(DatabaseError::Other(e)),
//             Error::StateRootError(e) => {
//                 ProviderError::Database(DatabaseError::Other(e.to_string()))
//             }
//             Error::RethStateRootError(e) => {
//                 ProviderError::Database(DatabaseError::Other(e.to_string()))
//             }
//             Error::ParallelStateRootError(e) => {
//                 ProviderError::Database(DatabaseError::Other(e.to_string()))
//             }
//             Error::BlockNotFound(_) => {
//                 ProviderError::Database(DatabaseError::Other(value.to_string()))
//             }
//         }
//     }
// }
