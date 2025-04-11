use crate::utils::config::{
    errors::{ConfigError, TomlParserError},
    ConfigMode,
};
use serde::Deserialize;
use std::fs::OpenOptions;
use std::io::Write;

use super::{
    block_producer::BlockProducerConfig, committer::CommitterConfig, deployer::DeployerConfig,
    eth::EthConfig, l1_watcher::L1WatcherConfig, prover_client::ProverClientConfig,
    prover_server::ProverServerConfig,
};

#[derive(Deserialize, Debug)]
struct L2Config {
    deployer: DeployerConfig,
    eth: EthConfig,
    watcher: L1WatcherConfig,
    proposer: BlockProducerConfig,
    committer: CommitterConfig,
    prover_server: ProverServerConfig,
}

impl L2Config {
    fn to_env(&self) -> String {
        let mut env_representation = String::new();

        env_representation.push_str(&self.deployer.to_env());
        env_representation.push_str(&self.eth.to_env());
        env_representation.push_str(&self.watcher.to_env());
        env_representation.push_str(&self.proposer.to_env());
        env_representation.push_str(&self.committer.to_env());
        env_representation.push_str(&self.prover_server.to_env());

        env_representation
    }

    fn write_env(&self) -> Result<(), TomlParserError> {
        let path = ConfigMode::Sequencer.get_env_path_or_default();
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| {
                TomlParserError::EnvWriteError(format!(
                    "Failed to open file {}: {e}",
                    path.to_str().unwrap_or("`Invalid path`")
                ))
            })?;

        file.write_all(&self.to_env().into_bytes())
            .map_err(|e| TomlParserError::EnvWriteError(e.to_string()))
    }
}

fn read_config(config_path: String, mode: ConfigMode) -> Result<(), ConfigError> {
    let toml_path = mode.get_config_file_path(&config_path);
    let toml_file_name = toml_path
        .file_name()
        .ok_or(ConfigError::Custom("Invalid CONFIGS_PATH".to_string()))?
        .to_str()
        .ok_or(ConfigError::Custom("Couldn't convert to_str()".to_string()))?
        .to_owned();
    let file = std::fs::read_to_string(toml_path).map_err(|err| {
        TomlParserError::TomlFileNotFound(format!("{err}: {}", toml_file_name.clone()), mode)
    })?;
    match mode {
        ConfigMode::Sequencer => {
            let config: L2Config = toml::from_str(&file).map_err(|err| {
                TomlParserError::TomlFormat(format!("{err}: {}", toml_file_name.clone()), mode)
            })?;
            config.write_env()?;
        }
        ConfigMode::ProverClient => {
            let config: ProverClientConfig = toml::from_str(&file).map_err(|err| {
                TomlParserError::TomlFormat(format!("{err}: {}", toml_file_name.clone()), mode)
            })?;
            config.write_env()?;
        }
    }

    Ok(())
}

pub fn parse_configs(mode: ConfigMode) -> Result<(), ConfigError> {
    #[allow(clippy::expect_fun_call, clippy::expect_used)]
    let config_path = std::env::var("CONFIGS_PATH").expect(
        format!(
            "CONFIGS_PATH environment variable not defined. Expected in {}, line: {}
If running locally, a reasonable value would be CONFIGS_PATH=./configs",
            file!(),
            line!()
        )
        .as_str(),
    );

    read_config(config_path, mode).map_err(From::from)
}
