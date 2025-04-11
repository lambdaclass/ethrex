use crate::utils::config::{errors::ConfigError, ConfigMode};

use super::{prover_client::ProverClientConfig, sequencer::SequencerConfig, L2Config};

fn read_config(config_path: String, mode: ConfigMode) -> Result<(), ConfigError> {
    let toml_path = mode.get_config_file_path(&config_path);
    match mode {
        ConfigMode::Sequencer => {
            let config = SequencerConfig::parse_toml(&toml_path)?;
            config.write_env(&mode.get_env_path_or_default())?;
        }
        ConfigMode::ProverClient => {
            let config = ProverClientConfig::parse_toml(&toml_path)?;
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
