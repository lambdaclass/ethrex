use ethereum_rust_storage::error::StoreError;
use revm::primitives::{
    result::EVMError as RevmError, Address as RevmAddress, B256 as RevmB256, U256 as RevmU256,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EvmError {
    #[error("Invalid Transaction: {0}")]
    Transaction(String),
    #[error("Invalid Header: {0}")]
    Header(String),
    #[error("DB error: {0}")]
    DB(#[from] StoreError),
    #[error("{0}")]
    Custom(String),
    #[error("{0}")]
    Precompile(String),
}

#[derive(Debug, Error)]
pub enum ExecutionDBError {
    #[error("Account {0} not found")]
    AccountNotFound(RevmAddress),
    #[error("Code by hash {0} not found")]
    CodeNotFound(RevmB256),
    #[error("Storage value for address {0} and slot {1} not found")]
    StorageNotFound(RevmAddress, RevmU256),
    #[error("Hash of block with number {0} not found")]
    BlockHashNotFound(u64),
    #[error("{0}")]
    Custom(String),
}

impl From<RevmError<StoreError>> for EvmError {
    fn from(value: RevmError<StoreError>) -> Self {
        match value {
            RevmError::Transaction(err) => EvmError::Transaction(err.to_string()),
            RevmError::Header(err) => EvmError::Header(err.to_string()),
            RevmError::Database(err) => EvmError::DB(err),
            RevmError::Custom(err) => EvmError::Custom(err),
            RevmError::Precompile(err) => EvmError::Precompile(err),
        }
    }
}
