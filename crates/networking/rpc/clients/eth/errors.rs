use crate::utils::RpcRequest;
use ethrex_common::{FromStrRadixErr, types::transaction::GenericTransactionError};

#[derive(Debug, thiserror::Error)]
pub enum EthClientError {
    #[error("Error sending request {0:?}")]
    RequestError(RpcRequest),
    #[error("reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("eth_gasPrice request error: {0}")]
    GetGasPriceError(#[from] GetGasPriceError),
    #[error("eth_estimateGas request error: {0}")]
    EstimateGasError(#[from] EstimateGasError),
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
    #[error("eth_getBlockByNumber request error: {0}")]
    GetBlockByNumberError(#[from] GetBlockByNumberError),
    #[error("net_peerCount request error: {0}")]
    GetPeerCountError(#[from] GetPeerCountError),
    #[error("debug_getRawBlock request error: {0}")]
    GetRawBlockError(#[from] GetRawBlockError),
    #[error("eth_getLogs request error: {0}")]
    GetLogsError(#[from] GetLogsError),
    #[error("eth_getTransactionReceipt request error: {0}")]
    GetTransactionReceiptError(#[from] GetTransactionReceiptError),
    #[error("Failed to serialize request body: {0}")]
    FailedToSerializeRequestBody(String),
    #[error("Failed to deserialize response body: {0}")]
    GetBalanceError(#[from] GetBalanceError),
    #[error("Failed to deserialize response body: {0}")]
    GetCodeError(#[from] GetCodeError),
    #[error("eth_getTransactionByHash request error: {0}")]
    GetTransactionByHashError(#[from] GetTransactionByHashError),
    #[error("ethrex_getWithdrawalProof request error: {0}")]
    GetMessageProofError(#[from] GetMessageProofError),
    #[error("debug_executionWitness request error: {0}")]
    GetWitnessError(#[from] GetWitnessError),
    #[error("eth_maxPriorityFeePerGas request error: {0}")]
    GetMaxPriorityFeeError(#[from] GetMaxPriorityFeeError),
    #[error("eth_config request error: {0}")]
    GetEthConfigError(#[from] GetEthConfigError),
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
    #[error("Failed to get transaction pool: {0}")]
    FailedToGetTxPool(#[from] TxPoolContentError),
    #[error("ethrex_getBatchByNumber request error: {0}")]
    GetBatchByNumberError(#[from] GetBatchByNumberError),
    #[error("ethrex_batchNumber request error: {0}")]
    GetBatchNumberError(#[from] GetBatchNumberError),
    #[error("ethrex_getBlobBaseFee request error: {0}")]
    GetBlobBaseFeeError(#[from] GetBlobBaseFeeRequestError),
    #[error("All RPC calls failed")]
    FailedAllRPC,
    #[error("Generic transaction error: {0}")]
    GenericTransactionError(#[from] GenericTransactionError),
    #[error("Failed to parse hex string: {0}")]
    FromStrRadixError(#[from] FromStrRadixErr),
    #[error("ethrex_getBaseFeeVaultAddress request error: {0}")]
    GetBaseFeeVaultAddressError(#[from] GetBaseFeeVaultAddressError),
    #[error("ethrex_getOperatorFeeVaultAddress request error: {0}")]
    GetOperatorFeeVaultAddressError(#[from] GetOperatorFeeVaultAddressError),
    #[error("ethrex_getOperatorFee request error: {0}")]
    GetOperatorFeeError(#[from] GetOperatorFeeError),
    #[error("ethrex_getL1FeeVaultAddress request error: {0}")]
    GetL1FeeVaultAddressError(#[from] GetL1FeeVaultAddressError),
    #[error("ethrex_getL1BlobBaseFee request error: {0}")]
    GetL1BlobBaseFeeError(#[from] GetL1BlobBaseFeeRequestError),
    #[error("ethrex_sendTransaction request error: {0}")]
    SendEthrexTransactionError(#[from] SendEthrexTransactionError),
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
pub enum EstimateGasError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
    #[error("{0}")]
    Custom(String),
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
pub enum GetBlockByNumberError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GetPeerCountError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GetRawBlockError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    RLPDecodeError(String),
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
pub enum GetCodeError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    NotHexError(#[from] hex::FromHexError),
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

#[derive(Debug, thiserror::Error)]
pub enum CalldataEncodeError {
    #[error("Failed to parse function signature: {0}")]
    ParseError(String),
    #[error("Wrong number of arguments provided for calldata: {0}")]
    WrongArgumentLength(String),
    #[error("Internal Calldata encoding error. This is most likely a bug")]
    InternalError,
}

// TODO: move to L2
#[derive(Debug, thiserror::Error)]
pub enum GetMessageProofError {
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
pub enum GetWitnessError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GetMaxPriorityFeeError {
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
pub enum TxPoolContentError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GetBlobBaseFeeRequestError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

// TODO: move to L2
#[derive(Debug, thiserror::Error)]
pub enum GetBatchByNumberError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GetBatchNumberError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[derive(Debug, thiserror::Error)]
pub enum GetEthConfigError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GetBaseFeeVaultAddressError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
}
#[derive(Debug, thiserror::Error)]
pub enum GetOperatorFeeVaultAddressError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
}
#[derive(Debug, thiserror::Error)]
pub enum GetOperatorFeeError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GetL1FeeVaultAddressError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GetL1BlobBaseFeeRequestError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[derive(Debug, thiserror::Error)]
pub enum SendEthrexTransactionError {
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseHashError(String),
}
