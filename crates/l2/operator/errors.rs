use crate::utils::{config::errors::ConfigError, eth_client::errors::EthClientError};
use ethereum_rust_dev::utils::engine_client::errors::EngineClientError;

#[derive(Debug, thiserror::Error)]
pub enum L1WatcherError {
    #[error("L1Watcher error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("L1Watcher failed to deserialize log: {0}")]
    FailedToDeserializeLog(String),
    #[error("L1Watcher failed to parse private key: {0}")]
    FailedToDeserializePrivateKey(String),
    #[error("L1Watcher failed to retrieve depositor account info: {0}")]
    FailedToRetrieveDepositorAccountInfo(String),
    #[error("L1Watcher failed to retrieve chain config: {0}")]
    FailedToRetrieveChainConfig(String),
    #[error("L1Watcher failed to get config: {0}")]
    FailedToGetConfig(#[from] ConfigError),
}

#[derive(Debug, thiserror::Error)]
pub enum ProofDataProviderError {
    #[error("ProofDataProvider connection failed: {0}")]
    ConnectionError(#[from] std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum OperatorError {
    #[error("Operator failed because of an EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Operator failed because of an EngineClient error: {0}")]
    EngineClientError(#[from] EngineClientError),
    #[error("Operator failed to produce block: {0}")]
    FailedToProduceBlock(String),
    #[error("Operator failed to prepare PayloadAttributes timestamp: {0}")]
    FailedToGetSystemTime(#[from] std::time::SystemTimeError),
    #[error("Operator failed to serialize block: {0}")]
    FailedToRetrieveBlockFromStorage(String),
}
