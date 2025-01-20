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
