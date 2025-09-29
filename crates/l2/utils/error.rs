use ethrex_blockchain::error::ChainError;
use ethrex_common::H256;
use ethrex_storage::error::StoreError;

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
    #[error("Internal error: {0}")]
    InternalError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum UtilsError {
    #[error("Unable to parse withdrawal_event_selector: {0}")]
    WithdrawalSelectorError(String),
}
