use thiserror::Error;
use tokio::task::JoinError;

#[derive(Debug, Error)]
pub enum NetworkingError {
    #[error("{0}")]
    ConnectionError(String),
    #[error(transparent)]
    JoinError(#[from] JoinError),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
}
