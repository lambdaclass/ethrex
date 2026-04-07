use ethrex_storage::error::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum ExplorerError {
    #[error("Store error: {0}")]
    Store(#[from] StoreError),

    #[error("Runtime error: {0}")]
    Runtime(String),
}
