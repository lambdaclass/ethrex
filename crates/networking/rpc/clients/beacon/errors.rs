#[derive(Debug, thiserror::Error)]
pub enum BeaconClientError {
    #[error("reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("Unreachable nonce")]
    UnrecheableNonce,
    #[error("Error: {0}")]
    Custom(String),
}
