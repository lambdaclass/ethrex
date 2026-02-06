//! Error types for the block builder.

use ethrex_blockchain::error::{ChainError, MempoolError};
use ethrex_storage::error::StoreError;
use ethrex_vm::EvmError;

/// Errors that can occur during block building.
#[derive(Debug, thiserror::Error)]
pub enum BlockBuilderError {
    #[error("Storage error: {0}")]
    Store(#[from] StoreError),

    #[error("Chain error: {0}")]
    Chain(#[from] ChainError),

    #[error("EVM error: {0}")]
    Evm(#[from] EvmError),

    #[error("Mempool error: {0}")]
    Mempool(#[from] MempoolError),

    #[error("Invalid transaction: {0}")]
    InvalidTransaction(String),

    #[error("Genesis error: {0}")]
    Genesis(String),

    #[error("Internal error: {0}")]
    Internal(String),
}
