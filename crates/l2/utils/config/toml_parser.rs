use crate::utils::config::{errors::ConfigError, ConfigMode};

use super::{prover_client::ProverClientConfig, sequencer::SequencerConfig};

pub fn parse_configs(mode: ConfigMode) -> Result<(), ConfigError> {
    match mode {
        ConfigMode::Sequencer => SequencerConfig::toml_to_env(),
        ConfigMode::ProverClient => ProverClientConfig::toml_to_env(),
    }
}
