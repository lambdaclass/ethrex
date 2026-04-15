use tonic::Status;

#[derive(Debug, thiserror::Error)]
pub enum CredibleLayerError {
    #[error("gRPC transport error: {0}")]
    Transport(#[from] tonic::transport::Error),
    #[error("gRPC status error: {0}")]
    Status(#[from] Status),
    #[error("Stream closed unexpectedly")]
    StreamClosed,
    #[error("Result timeout for tx {0}")]
    ResultTimeout(String),
    #[error("Credible Layer error: {0}")]
    Internal(String),
}
