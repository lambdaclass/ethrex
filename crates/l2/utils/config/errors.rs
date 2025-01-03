use crate::proposer::errors::ProposerError;
use ethrex_dev::utils::engine_client;
use ethrex_l2_sdk::eth_client::errors::EthClientError;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Error deserializing config from env: {0}")]
    ConfigDeserializationError(#[from] envy::Error),
    #[error("Error reading env file: {0}")]
    EnvFileError(#[from] std::io::Error),
    #[error("Error building Proposer from config: {0}")]
    BuildProposerFromConfigError(#[from] ProposerError),
    #[error("Error building Proposer Engine from config: {0}")]
    BuildProposerEngineServerFromConfigError(#[from] engine_client::errors::ConfigError),
    #[error("Error building Prover server from config: {0}")]
    BuildProverServerFromConfigError(#[from] EthClientError),
    #[error("{0}")]
    Custom(String),
}
