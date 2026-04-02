use crate::utils::RpcRequest;
use ethrex_common::{FromStrRadixErr, types::transaction::GenericTransactionError};

/// A single error type for all RPC request failures.
#[derive(Debug, thiserror::Error)]
pub enum RpcRequestError {
    #[error("{method}: {source}")]
    SerdeJSONError {
        method: String,
        source: serde_json::Error,
    },
    #[error("{method}: {message} (data: {data:?})")]
    RPCError {
        method: String,
        message: String,
        data: Option<String>,
    },
    #[error("{method}: {source}")]
    ParseIntError {
        method: String,
        source: std::num::ParseIntError,
    },
    #[error("{method}: {source}")]
    HexError {
        method: String,
        source: hex::FromHexError,
    },
    #[error("{method}: {message}")]
    RLPDecodeError { method: String, message: String },
    #[error("{0}")]
    Custom(String),
}

#[derive(Debug, thiserror::Error)]
pub enum EthClientError {
    #[error("Error sending request {0:?}")]
    RequestError(RpcRequest),
    #[error("reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("RPC request error: {0}")]
    RpcRequestError(#[from] RpcRequestError),
    #[error("Failed to serialize request body: {0}")]
    FailedToSerializeRequestBody(String),
    #[error("Unreachable nonce")]
    UnreachableNonce,
    #[error("Error: {0}")]
    Custom(String),
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("Max number of retries reached when trying to send transaction")]
    TimeoutError,
    #[error("Internal Error. This is most likely a bug: {0}")]
    InternalError(String),
    #[error("Parse Url Error. {0}")]
    ParseUrlError(String),
    #[error("Failed to sign payload: {0}")]
    FailedToSignPayload(String),
    #[error("All RPC calls failed")]
    FailedAllRPC,
    #[error("Generic transaction error: {0}")]
    GenericTransactionError(#[from] GenericTransactionError),
    #[error("Failed to parse hex string: {0}")]
    FromStrRadixError(#[from] FromStrRadixErr),
}

#[derive(Debug, thiserror::Error)]
pub enum CalldataEncodeError {
    #[error("Failed to parse function signature: {0}")]
    ParseError(String),
    #[error("Wrong number of arguments provided for calldata: {0}")]
    WrongArgumentLength(String),
    #[error("Internal Calldata encoding error. This is most likely a bug")]
    InternalError,
}
