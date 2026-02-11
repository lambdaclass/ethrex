use ethrex_blockchain::error::{ChainError, MempoolError};
use ethrex_storage::error::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum BlockBuilderError {
    #[error("Storage error: {0}")]
    Store(#[from] StoreError),

    #[error("Chain error: {0}")]
    Chain(#[from] ChainError),

    #[error("Mempool error: {0}")]
    Mempool(#[from] MempoolError),

    #[error("Genesis error: {0}")]
    Genesis(String),

    #[error("Internal error: {0}")]
    Internal(String),
}
