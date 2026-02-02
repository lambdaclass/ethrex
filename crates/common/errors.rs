use crate::types::{InvalidBlockBodyError, InvalidBlockHeaderError};

#[derive(thiserror::Error, Debug)]
pub enum EcdsaError {
    #[cfg(all(
        not(feature = "zisk"),
        not(feature = "risc0"),
        not(feature = "sp1"),
        feature = "secp256k1"
    ))]
    #[error("secp256k1 error: {0}")]
    Secp256k1(#[from] secp256k1::Error),
    #[cfg(any(
        feature = "zisk",
        feature = "risc0",
        feature = "sp1",
        not(feature = "secp256k1")
    ))]
    #[error("k256 error: {0}")]
    K256(#[from] k256::ecdsa::Error),
}

/// Errors that occur during block validation.
///
/// These are validation errors that don't require storage access to detect.
#[derive(Debug, thiserror::Error)]
pub enum InvalidBlockError {
    #[error("Requests hash does not match the one in the header after executing")]
    RequestsHashMismatch,
    #[error("Block access list hash does not match the one in the header after executing")]
    BlockAccessListHashMismatch,
    #[error("Block access list contains index {index} exceeding max valid index {max}")]
    BlockAccessListIndexOutOfBounds { index: u16, max: u16 },
    #[error("World State Root does not match the one in the header after executing")]
    StateRootMismatch,
    #[error("Receipts Root does not match the one in the header after executing")]
    ReceiptsRootMismatch,
    #[error("Invalid Header, validation failed pre-execution: {0}")]
    InvalidHeader(#[from] InvalidBlockHeaderError),
    #[error("Invalid Body, validation failed pre-execution: {0}")]
    InvalidBody(#[from] InvalidBlockBodyError),
    #[error("Exceeded MAX_BLOB_GAS_PER_BLOCK")]
    ExceededMaxBlobGasPerBlock,
    #[error("Exceeded MAX_BLOB_NUMBER_PER_BLOCK")]
    ExceededMaxBlobNumberPerBlock,
    #[error("Gas used doesn't match value in header. Used: {0}, Expected: {1}")]
    GasUsedMismatch(u64, u64),
    #[error("Blob gas used doesn't match value in header")]
    BlobGasUsedMismatch,
    #[error("Invalid transaction: {0}")]
    InvalidTransaction(String),
    #[error("Maximum block size exceeded: Maximum is {0} MiB, but block was {1} MiB")]
    MaximumRlpSizeExceeded(u64, u64),
    #[error("Invalid block fork")]
    InvalidBlockFork,
}
