use ethrex_blockchain::error::ChainError;
use ethrex_storage::error::StoreError;
use ethrex_vm::ProverDBError;
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
    #[error("ProverDB error: {0}")]
    ProverDBError(#[from] ProverDBError),
    #[error("Internal error: {0}")]
    InternalError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum UtilsError {
    #[error("Unable to parse withdrawal_event_selector: {0}")]
    WithdrawalSelectorError(String),
}
