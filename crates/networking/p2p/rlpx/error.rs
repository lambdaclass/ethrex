use ethrex_rlp::error::{RLPDecodeError, RLPEncodeError};
use ethrex_storage::error::StoreError;
use thiserror::Error;
use tokio::sync::broadcast::error::RecvError;

// TODO improve errors
#[derive(Debug, Error)]
pub(crate) enum RLPxError {
    #[error("{0}")]
    HandshakeError(String),
    #[error("{0}")]
    ConnectionError(String),
    #[error("Invalid connection state")]
    InvalidState(),
    #[error("Disconnect received")]
    Disconnect(),
    #[error("Not Found: {0}")]
    NotFound(String),
    #[error("Invalid peer id")]
    InvalidPeerId(),
    #[error("Invalid recovery id")]
    InvalidRecoveryId(),
    #[error("Invalid message length")]
    InvalidMessageLength(),
    #[error("Cannot handle message: {0}")]
    MessageNotHandled(String),
    #[error("Bad Request: {0}")]
    BadRequest(String),
    #[error(transparent)]
    RLPDecodeError(#[from] RLPDecodeError),
    #[error(transparent)]
    RLPEncodeError(#[from] RLPEncodeError),
    #[error(transparent)]
    StoreError(#[from] StoreError),
    #[error("Error in cryptographic library: {0}")]
    CryptographyError(String),
    #[error("Failed to broadcast msg: {0}")]
    BroadcastError(String),
    #[error(transparent)]
    RecvError(#[from] RecvError),
}

// Grouping all cryptographic related errors in a single CryptographicError variant
// We can improve this to individual errors if required
impl From<k256::ecdsa::Error> for RLPxError {
    fn from(e: k256::ecdsa::Error) -> Self {
        RLPxError::CryptographyError(e.to_string())
    }
}

impl From<k256::elliptic_curve::Error> for RLPxError {
    fn from(e: k256::elliptic_curve::Error) -> Self {
        RLPxError::CryptographyError(e.to_string())
    }
}

impl From<sha3::digest::InvalidLength> for RLPxError {
    fn from(e: sha3::digest::InvalidLength) -> Self {
        RLPxError::CryptographyError(e.to_string())
    }
}

impl From<aes::cipher::StreamCipherError> for RLPxError {
    fn from(e: aes::cipher::StreamCipherError) -> Self {
        RLPxError::CryptographyError(e.to_string())
    }
}
