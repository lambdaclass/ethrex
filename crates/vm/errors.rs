use std::fmt::Display;

use ethereum_types::{H160, H256};
use ethrex_common::{Address, types::BlockHash};
use ethrex_levm::errors::{DatabaseError as LevmDatabaseError, InternalError, VMError};
use ethrex_trie::TrieError;
#[cfg(feature = "revm")]
use revm::primitives::result::EVMError as RevmError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EvmError {
    #[error("Invalid Transaction: {0}")]
    Transaction(String),
    #[error("Invalid Header: {0}")]
    Header(String),
    #[error("DB error: {0}")]
    DB(String),
    #[error("{0}")]
    Precompile(String),
    #[error("Invalid EVM or EVM not supported: {0}")]
    InvalidEVM(String),
    #[error("{0}")]
    Custom(String),
    #[error("Invalid deposit request layout")]
    InvalidDepositRequest,
    #[error("System call failed: {0}")]
    SystemContractCallFailed(String),
}

#[cfg(feature = "revm")]
impl<E: Display> From<RevmError<E>> for EvmError {
    fn from(value: RevmError<E>) -> Self {
        match value {
            RevmError::Transaction(err) => EvmError::Transaction(err.to_string()),
            RevmError::Header(err) => EvmError::Header(err.to_string()),
            RevmError::Database(err) => EvmError::DB(err.to_string()),
            RevmError::Custom(err) => EvmError::Custom(err),
            RevmError::Precompile(err) => EvmError::Precompile(err),
        }
    }
}

impl From<VMError> for EvmError {
    fn from(value: VMError) -> Self {
        if value.should_propagate() {
            EvmError::Custom(value.to_string())
        } else {
            // If an error is not internal it means it is a transaction validation error.
            EvmError::Transaction(value.to_string())
        }
    }
}

impl From<LevmDatabaseError> for EvmError {
    fn from(value: LevmDatabaseError) -> Self {
        EvmError::DB(value.to_string())
    }
}

impl From<InternalError> for EvmError {
    fn from(value: InternalError) -> Self {
        match value {
            InternalError::Database(err) => err.into(),
            other => EvmError::Custom(other.to_string()),
        }
    }
}
