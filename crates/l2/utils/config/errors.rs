use crate::{sequencer::errors::BlockProducerError, utils::config::ConfigMode};
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
    #[error("Error parsing the .toml configuration files: {0}")]
    TomlParserError(#[from] TomlParserError),
    #[error("{0}")]
    Custom(String),
}

#[derive(Debug, thiserror::Error)]
pub enum TomlParserError {
    #[error(
        "Could not find crates/l2/configs/{0}
Have you tried copying the provided example? Try:
cp {manifest_dir}/configs/{1}_config_example.toml {manifest_dir}/configs/{1}_config.toml
",
        manifest_dir = env!("CARGO_MANIFEST_DIR")

    )]
    TomlFileNotFound(String, ConfigMode),

    #[error(
        "Could not parse crates/l2/configs/{0}
Check the provided example to see if you have all the required fields.
The example can be found at:
crates/l2/configs/{1}_config_example.toml
You can also see the differences with:
diff {manifest_dir}/configs/{1}_config_example.toml {manifest_dir}/configs/{1}_config.toml
",
        manifest_dir = env!("CARGO_MANIFEST_DIR")

    )]
    TomlFormat(String, ConfigMode),

    #[error("\x1b[91mCould not write to .env file.\x1b[0m {0}")]
    EnvWriteError(String),
}
