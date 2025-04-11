use crate::utils::config::{
    errors::{ConfigError, TomlParserError},
    ConfigMode,
};

use super::{prover_client::ProverClientConfig, sequencer::SequencerConfig, L2Config};

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
            let config: SequencerConfig = toml::from_str(&file).map_err(|err| {
                TomlParserError::TomlFormat(format!("{err}: {}", toml_file_name.clone()), mode)
            })?;
            config.write_env(&mode.get_env_path_or_default())?;
        }
        ConfigMode::ProverClient => {
            let config: ProverClientConfig = toml::from_str(&file).map_err(|err| {
                TomlParserError::TomlFormat(format!("{err}: {}", toml_file_name.clone()), mode)
            })?;
            config.write_env(&mode.get_env_path_or_default())?;
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
