use ethrex_rpc::utils::RpcRequest;

#[derive(Debug, thiserror::Error)]
pub enum EthClientError {
    #[error("Error sending request {0:?}")]
    RequestError(RpcRequest),
    #[error("reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("eth_gasPrice request error: {0}")]
    GetGasPriceError(#[from] GetGasPriceError),
    #[error("eth_estimateGas request error: {0}")]
    EstimateGasPriceError(#[from] EstimateGasPriceError),
    #[error("eth_sendRawTransaction request error: {0}")]
    SendRawTransactionError(#[from] SendRawTransactionError),
    #[error("eth_call request error: {0}")]
    CallError(#[from] CallError),
    #[error("eth_getTransactionCount request error: {0}")]
    GetNonceError(#[from] GetNonceError),
    #[error("eth_blockNumber request error: {0}")]
    GetBlockNumberError(#[from] GetBlockNumberError),
    #[error("eth_getBlockByHash request error: {0}")]
    GetBlockByHashError(#[from] GetBlockByHashError),
    #[error("eth_getLogs request error: {0}")]
    GetLogsError(#[from] GetLogsError),
    #[error("eth_getTransactionReceipt request error: {0}")]
    GetTransactionReceiptError(#[from] GetTransactionReceiptError),
    #[error("Failed to serialize request body: {0}")]
    FailedToSerializeRequestBody(String),
    #[error("Failed to deserialize response body: {0}")]
    GetBalanceError(#[from] GetBalanceError),
    #[error("eth_getTransactionByHash request error: {0}")]
    GetTransactionByHashError(#[from] GetTransactionByHashError),
    #[error("Unreachable nonce")]
    UnrecheableNonce,
    #[error("Error: {0}")]
    Custom(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GetGasPriceError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[derive(Debug, thiserror::Error)]
pub enum EstimateGasPriceError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[derive(Debug, thiserror::Error)]
pub enum SendRawTransactionError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[derive(Debug, thiserror::Error)]
pub enum GetNonceError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[derive(Debug, thiserror::Error)]
pub enum GetBlockNumberError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[derive(Debug, thiserror::Error)]
pub enum GetBlockByHashError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[derive(Debug, thiserror::Error)]
pub enum GetLogsError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[derive(Debug, thiserror::Error)]
pub enum GetTransactionReceiptError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[derive(Debug, thiserror::Error)]
pub enum GetBalanceError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[derive(Debug, thiserror::Error)]
pub enum GetTransactionByHashError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}
