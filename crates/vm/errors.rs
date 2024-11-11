use ethereum_rust_core::types::BlockHash;
use ethereum_rust_storage::error::StoreError;
use ethereum_rust_trie::TrieError;
use ethereum_types::H160;
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
    #[error("Execution DB error: {0}")]
    ExecutionDB(#[from] ExecutionDBError),
    #[error("{0}")]
    Custom(String),
    #[error("{0}")]
    Precompile(String),
}

#[derive(Debug, Error)]
pub enum ExecutionDBError {
    #[error("Store error: {0}")]
    Store(#[from] StoreError),
    #[error("Evm error: {0}")]
    Evm(#[from] Box<EvmError>), // boxed to avoid cyclic definition
    #[error("Trie error: {0}")]
    Trie(#[from] TrieError),
    #[error("State proofs error: {0}")]
    StateProofs(#[from] StateProofsError),
    #[error("Account {0} not found")]
    AccountNotFound(RevmAddress),
    #[error("Code by hash {0} not found")]
    CodeNotFound(RevmB256),
    #[error("Storage value for address {0} and slot {1} not found")]
    StorageNotFound(RevmAddress, RevmU256),
    #[error("Hash of block with number {0} not found")]
    BlockHashNotFound(u64),
    #[error("Missing account {0} info while trying to create ExecutionDB")]
    NewMissingAccountInfo(RevmAddress),
    #[error("Missing state trie of block {0} while trying to create ExecutionDB")]
    NewMissingStateTrie(BlockHash),
    #[error(
        "Missing storage trie of block {0} and address {1} while trying to create ExecutionDB"
    )]
    NewMissingStorageTrie(BlockHash, RevmAddress),
    #[error("{0}")]
    Custom(String),
}

#[derive(Debug, Error)]
pub enum StateProofsError {
    #[error("Trie error: {0}")]
    Trie(#[from] TrieError),
    #[error("Storage trie for address {0} not found")]
    StorageTrieNotFound(H160),
    #[error("Storage for address {0} not found")]
    StorageNotFound(RevmAddress),
    #[error("Account proof for address {0} not found")]
    AccountProofNotFound(RevmAddress),
    #[error("Storage proofs for address {0} not found")]
    StorageProofsNotFound(RevmAddress),
    #[error("Storage proof for address {0} and key {1} not found")]
    StorageProofNotFound(RevmAddress, RevmU256),
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

impl From<RevmError<ExecutionDBError>> for EvmError {
    fn from(value: RevmError<ExecutionDBError>) -> Self {
        match value {
            RevmError::Transaction(err) => EvmError::Transaction(err.to_string()),
            RevmError::Header(err) => EvmError::Header(err.to_string()),
            RevmError::Database(err) => EvmError::ExecutionDB(err),
            RevmError::Custom(err) => EvmError::Custom(err),
            RevmError::Precompile(err) => EvmError::Precompile(err),
        }
    }
}
