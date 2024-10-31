use crate::rlpx::message::Message;
use ethereum_rust_storage::error::StoreError;
use thiserror::Error;

// TODO improve errors
#[derive(Debug, Error)]
pub(crate) enum RLPxError {
    #[error("{0}")]
    HandshakeError(String),
    #[error("{0}")]
    InvalidState(String),
    #[error("Unexpected message: {0}")]
    UnexpectedMessage(Message),
    #[error(transparent)]
    Store(#[from] StoreError),
}
