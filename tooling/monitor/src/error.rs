use ethrex_common::Address;
use ethrex_rpc::clients::eth::errors::{CalldataEncodeError, EthClientError};
use ethrex_storage::error::StoreError;
use ethrex_storage_rollup::RollupStoreError;
use spawned_concurrency::error::GenServerError;

#[derive(Debug, thiserror::Error)]
pub enum MonitorError {
    #[error("Failed because of io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to fetch {0:?} logs from {1}, {2}")]
    LogsSignatures(Vec<String>, Address, #[source] EthClientError),
    #[error("Failed to get batch by number {0}: {1}")]
    GetBatchByNumber(u64, #[source] RollupStoreError),
    #[error("Failed to get blocks by batch number {0}: {1}")]
    GetBlocksByBatch(u64, #[source] RollupStoreError),
    #[error("Batch {0} not found in the rollup store")]
    BatchNotFound(u64),
    #[error("Failed to get block by number {0}, {1}")]
    GetBlockByNumber(u64, #[source] StoreError),
    #[error("Block {0} not found in the store")]
    BlockNotFound(u64),
    #[error("Internal Error: {0}")]
    InternalError(#[from] GenServerError),
    #[error("Failed to get logs topics {0}")]
    LogsTopics(usize),
    #[error("Failed to get logs data from {0}")]
    LogsData(usize),
    #[error("Failed to get area chunks")]
    Chunks,
    #[error("Failed to get latest block")]
    GetLatestBlock,
    #[error("Failed to get latest batch")]
    GetLatestBatch,
    #[error("Failed to get latest verified batch")]
    GetLatestVerifiedBatch,
    #[error("Failed to get committed batch")]
    GetLatestCommittedBatch,
    #[error("Failed to get last L1 block fetched")]
    GetLastFetchedL1,
    #[error("Failed to get pending privileged transactions")]
    GetPendingPrivilegedTx,
    #[error("Failed to get transaction pool")]
    TxPoolError,
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("Failed to parse privileged transaction")]
    PrivilegedTxParseError,
    #[error("Failure in rpc call: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Failed to get receipt for transaction")]
    ReceiptError,
    #[error("Expected transaction to have logs")]
    NoLogs,
    #[error("Expected items in the table")]
    NoItemsInTable,
    #[error("RPC List can't be empty")]
    RPCListEmpty,
    #[error("Error converting batch window")]
    BatchWindow,
    #[error("Error while parsing private key")]
    DecodingError(String),
    #[error("Error parsing secret key")]
    FromHexError(#[from] hex::FromHexError),
}
