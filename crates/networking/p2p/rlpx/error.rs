use thiserror::Error;

// TODO improve errors
#[derive(Debug, Error)]
pub enum RLPxError {
    #[error("{0}")]
    HandshakeError(String),
}
