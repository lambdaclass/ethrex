use alloc::string::String;
use thiserror::Error;

// TODO: improve errors
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RLPDecodeError {
    #[error("InvalidLength")]
    InvalidLength,
    #[error("MalformedData")]
    MalformedData,
    #[error("MalformedBoolean")]
    MalformedBoolean,
    #[error("UnexpectedList")]
    UnexpectedList,
    #[error("UnexpectedString")]
    UnexpectedString,
    #[error("InvalidCompression: {0}")]
    InvalidCompression(String),
    #[error("IncompatibleProtocol: {0}")]
    IncompatibleProtocol(String),
    #[error("{0}")]
    Custom(String),
}

// TODO: improve errors
#[derive(Debug, Error)]
pub enum RLPEncodeError {
    #[error("InvalidCompression: {0}")]
    InvalidCompression(String),
    #[error("{0}")]
    Custom(String),
}
