use crate::sequencer::errors::BlockProducerError;
use ethrex_rpc::clients::{auth, eth};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Error deserializing config from env: {err}. From config: {from:?}")]
    ConfigDeserializationError { err: envy::Error, from: String },
    #[error("Error reading env file: {0}")]
    EnvFileError(#[from] std::io::Error),
    #[error("Error building Proposer from config: {0}")]
    BuildBlockProducerFromConfigError(#[from] BlockProducerError),
    #[error("Error building Proposer Engine from config: {0}")]
    BuildProposerEngineServerFromConfigError(#[from] auth::errors::ConfigError),
    #[error("Error building Prover server from config: {0}")]
    BuildProverServerFromConfigError(#[from] eth::errors::EthClientError),
    #[error("Error parsing '{0}' as hex value")]
    HexParsingError(String),
    #[error("{0}")]
    Custom(String),
}
