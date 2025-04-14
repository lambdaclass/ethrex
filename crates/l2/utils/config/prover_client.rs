use std::path::PathBuf;

use crate::utils::parse::url_deserializer;
use reqwest::Url;
use serde::Deserialize;
use tracing::{info, warn};

use super::{errors::ConfigError, ConfigMode, L2Config};

#[derive(Deserialize, Debug)]
pub struct ProverClientConfig {
    #[serde(deserialize_with = "url_deserializer")]
    pub prover_server_endpoint: Url,
    pub proving_time_ms: u64,
}

impl L2Config for ProverClientConfig {
    const PREFIX: &str = "PROVER_CLIENT_";

    fn to_env(&self) -> String {
        format!(
            "
{prefix}_PROVER_SERVER_ENDPOINT={}
{prefix}_PROVING_TIME_MS={}
",
            self.prover_server_endpoint,
            self.proving_time_ms,
            prefix = Self::PREFIX
        )
    }
}

impl ProverClientConfig {
    pub fn toml_to_env() -> Result<(), ConfigError> {
        let configs_path = std::env::var("CONFIGS_PATH")
            .map_err(|_| ConfigError::EnvNotFound("CONFIGS_PATH".to_string()))?;
        let config =
            Self::parse_toml(&ConfigMode::ProverClient.get_config_file_path(&configs_path))?;
        config.write_env(&ConfigMode::ProverClient.get_env_path_or_default())
    }

    /// Load the prover client config.
    /// If the `ETHREX_PROVER_CONFIG` env var is set, the config is parsed from that file.
    /// Else, the config is tried to be getted from the environment.
    pub fn load() -> Result<Self, ConfigError> {
        match std::env::var("ETHREX_PROVER_CONFIG") {
            Ok(config_path) => {
                info!("Reading config from TOML file: {config_path}");
                Self::parse_toml(&PathBuf::from(&config_path))
            }
            Err(_) => {
                warn!("ETHREX_PROVER_CONFIG env var not set. Reading config from environment");
                Self::from_env()
            }
        }
    }
}
