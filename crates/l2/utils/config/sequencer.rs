use serde::Deserialize;

use super::{
    block_producer::BlockProducerConfig, committer::CommitterConfig, deployer::DeployerConfig,
    errors::ConfigError, eth::EthConfig, l1_watcher::L1WatcherConfig,
    prover_server::ProverServerConfig, ConfigMode, L2Config,
};

#[derive(Deserialize, Debug)]
pub struct SequencerConfig {
    pub deployer: DeployerConfig,
    pub eth: EthConfig,
    pub watcher: L1WatcherConfig,
    pub block_producer: BlockProducerConfig,
    pub committer: CommitterConfig,
    pub prover_server: ProverServerConfig,
}

impl L2Config for SequencerConfig {
    const PREFIX: &str = "";

    fn to_env(&self) -> String {
        let mut env_representation = String::new();

        env_representation.push_str(&self.deployer.to_env());
        env_representation.push_str(&self.eth.to_env());
        env_representation.push_str(&self.watcher.to_env());
        env_representation.push_str(&self.block_producer.to_env());
        env_representation.push_str(&self.committer.to_env());
        env_representation.push_str(&self.prover_server.to_env());

        env_representation
    }
}

impl SequencerConfig {
    pub fn toml_to_env() -> Result<(), ConfigError> {
        let configs_path = std::env::var("CONFIGS_PATH")
            .map_err(|_| ConfigError::EnvNotFound("CONFIGS_PATH".to_string()))?;
        let config = Self::parse_toml(&ConfigMode::Sequencer.get_config_file_path(&configs_path))?;
        config.write_env(&ConfigMode::Sequencer.get_env_path_or_default())
    }

    pub fn load() -> Result<Self, ConfigError> {
        let configs_path = std::env::var("CONFIGS_PATH")
            .map_err(|_| ConfigError::EnvNotFound("CONFIGS_PATH".to_string()))?;
        Self::from_env().or(Self::parse_toml(
            &ConfigMode::Sequencer.get_config_file_path(&configs_path),
        ))
    }
}
