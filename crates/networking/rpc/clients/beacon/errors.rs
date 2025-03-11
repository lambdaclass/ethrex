#[derive(Debug, thiserror::Error)]
pub enum BeaconClientError {
    #[error("reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("Beacon RPC error (code: {0}): {1}")]
    RpcError(u64, String),
    #[error("Response deserialization error: {0}")]
    DeserializeError(#[from] serde_json::Error),
    #[error("Error: {0}")]
    Custom(String),
}
