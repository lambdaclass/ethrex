//! Error type for the t8n tool.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum T8nError {
    #[error("unsupported fork: {0}")]
    UnsupportedFork(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("failed to read {0}: {1}")]
    Io(String, std::io::Error),
    #[error("failed to parse {0}: {1}")]
    Parse(String, String),
    #[error("store error: {0}")]
    Store(String),
    #[error("block build error: {0}")]
    Build(String),
}
