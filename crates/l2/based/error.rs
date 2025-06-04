use ethrex_blockchain::error::{ChainError, InvalidForkChoice};
use ethrex_rlp::error::RLPDecodeError;
use ethrex_rpc::clients::{eth::errors::CalldataEncodeError, EthClientError};
use ethrex_storage::error::StoreError;
use ethrex_vm::EvmError;

use crate::{sequencer::errors::ExecutionCacheError, utils::error::UtilsError};

#[derive(Debug, thiserror::Error)]
pub enum StateUpdaterError {
    #[error("State Updater failed due to an EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("State Updater failed when trying to encode the calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("State Updater failed when trying to parse the calldata: {0}")]
    CalldataParsingError(String),
    #[error("State Updater failed due to a Store error: {0}")]
    StoreError(#[from] StoreError),
    #[error("Failed to apply fork choice for fetched block: {0}")]
    InvalidForkChoice(#[from] InvalidForkChoice),
    #[error("Internal Error: {0}")]
    InternalError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum BlockFetcherError {
    #[error("Block Fetcher failed due to an EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Block Fetcher failed due to a Store error: {0}")]
    StoreError(#[from] StoreError),
    #[error("Internal Error: {0}")]
    InternalError(String),
    #[error("Failed to store fetched block: {0}")]
    ChainError(#[from] ChainError),
    #[error("Failed to apply fork choice for fetched block: {0}")]
    InvalidForkChoice(#[from] InvalidForkChoice),
    #[error("Failed to push fetched block to execution cache: {0}")]
    ExecutionCacheError(#[from] ExecutionCacheError),
    #[error("Failed to RLP decode fetched block: {0}")]
    RLPDecodeError(#[from] RLPDecodeError),
    #[error("Block Fetcher failed in a helper function: {0}")]
    UtilsError(#[from] UtilsError),
    #[error("Missing bytes from calldata: {0}")]
    WrongBatchCalldata(String),
    #[error("Failed due to an EVM error: {0}")]
    EvmError(#[from] EvmError),
    #[error("Failed to produce the blob bundle")]
    BlobBundleError,
}
