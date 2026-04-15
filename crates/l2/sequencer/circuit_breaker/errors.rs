use tonic::Status;

#[derive(Debug, thiserror::Error)]
pub enum CircuitBreakerError {
    #[error("gRPC transport error: {0}")]
    Transport(#[from] tonic::transport::Error),
    #[error("gRPC status error: {0}")]
    Status(#[from] Status),
    #[error("Stream closed unexpectedly")]
    StreamClosed,
    #[error("Result timeout for tx {0}")]
    ResultTimeout(String),
    #[error("Circuit Breaker error: {0}")]
    Internal(String),
}
