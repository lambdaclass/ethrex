use std::fmt::Display;

use ethereum_types::{H160, H256};
use ethrex_common::{Address, types::BlockHash};
use ethrex_levm::errors::{DatabaseError as LevmDatabaseError, InternalError, VMError};
use ethrex_trie::TrieError;
#[cfg(feature = "revm")]
use revm::primitives::{
    Address as RevmAddress, B256 as RevmB256, U256 as RevmU256, result::EVMError as RevmError,
};
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

#[derive(Debug, Error)]
pub enum ProverDBError {
    #[error("Database error: {0}")]
    Database(String),
    #[error("Store error: {0}")]
    Store(String),
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
    #[error("Storage for address {0} not found")]
    StorageNotFound(RevmAddress),
    #[error("Storage value for address {0} and key {1} not found")]
    StorageValueNotFound(RevmAddress, RevmU256),
    #[error("Hash of block with number {0} not found")]
    BlockHashNotFound(u64),
    #[error("Missing state trie of block {0} while trying to create ProverDB")]
    NewMissingStateTrie(BlockHash),
    #[error("Missing storage trie of block {0} and address {1} while trying to create ProverDB")]
    NewMissingStorageTrie(BlockHash, Address),
    #[error("Missing account {0} info while trying to create ProverDB")]
    NewMissingAccountInfo(Address),
    #[error("Missing storage of address {0} and key {1} while trying to create ProverDB")]
    NewMissingStorage(Address, H256),
    #[error("Missing code of hash {0} while trying to create ProverDB")]
    NewMissingCode(H256),
    #[error("The account {0} is not included in the stored pruned state trie")]
    MissingAccountInStateTrie(H160),
    #[error("Missing storage trie of account {0}")]
    MissingStorageTrie(H160),
    #[error("Storage trie root for account {0} does not match account storage root")]
    InvalidStorageTrieRoot(H160),
    #[error("The pruned storage trie of account {0} is missing the storage key {1}")]
    MissingKeyInStorageTrie(H160, H256),
    #[error("Storage trie value for account {0} and key {1} does not match value stored in db")]
    InvalidStorageTrieValue(H160, H256),
    #[error("{0}")]
    Custom(String),
    #[error("No block headers stored, should at least store parent header")]
    NoBlockHeaders,
    #[error("Non-contiguous block headers (there's a gap in the block headers list)")]
    NoncontiguousBlockHeaders,
    #[error("Unreachable code reached: {0}")]
    Unreachable(String),
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
