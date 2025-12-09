use ethrex_blockchain::error::ChainError;
use ethrex_common::{H256, types::BlobsBundleError};
use ethrex_l2_common::privileged_transactions::PrivilegedTransactionError;
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
    #[error("Failed to retrieve data: {0}")]
    RetrievalError(String),
    #[error("Inconsistent Storage: {0}")]
    InconsistentStorage(String),
    #[error("Conversion Error: {0}")]
    ConversionError(String),
    #[error("Failed due to a Store error: {0}")]
    StoreError(#[from] ethrex_storage::error::StoreError),
    #[error("Failed to produce the blob bundle")]
    BlobBundleError(#[from] BlobsBundleError),
    #[error("Failed to compute deposit logs hash: {0}")]
    PrivilegedTransactionError(#[from] PrivilegedTransactionError),
    #[error("Privileged transaction hash could not be computed")]
    InvalidPrivilegedTransaction,
    #[error("Integer conversion error: {0}")]
    TryFromIntError(#[from] std::num::TryFromIntError),
}
