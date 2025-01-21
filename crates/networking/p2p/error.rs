use crate::rlpx::error::RLPxError;
use std::time::SystemTimeError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetworkingError {
    #[error("{0}")]
    ConnectionError(String),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error(transparent)]
    SystemTimeError(#[from] SystemTimeError),
}

// Don't want to export RLPxErrors outside this crate,
// So we just display the message
impl From<RLPxError> for NetworkingError {
    fn from(value: RLPxError) -> Self {
        Self::ConnectionError(value.to_string())
    }
}
