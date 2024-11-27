use ethrex_blockchain::error::ChainError;
use ethrex_storage::error::StoreError;
use ethrex_vm::errors::ExecutionDBError;
use keccak_hash::H256;

#[derive(Debug, thiserror::Error)]
pub enum ProverInputError {
    #[error("Invalid block number: {0}")]
    InvalidBlockNumber(usize),
    #[error("Invalid parent block: {0}")]
    InvalidParentBlock(H256),
    #[error("Store error: {0}")]
    StoreError(#[from] StoreError),
    #[error("Chain error: {0}")]
    ChainError(#[from] ChainError),
    #[error("ExecutionDB error: {0}")]
    ExecutionDBError(#[from] ExecutionDBError),
}
