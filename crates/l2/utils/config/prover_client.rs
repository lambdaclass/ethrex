use std::{fs::OpenOptions, io::Write};

use crate::utils::parse::url_deserializer;
use reqwest::Url;
use serde::Deserialize;

use super::{
    errors::{ConfigError, TomlParserError},
    ConfigMode,
};

pub const PROVER_CLIENT_PREFIX: &str = "PROVER_CLIENT_";

#[derive(Deserialize, Debug)]
pub struct ProverClientConfig {
    #[serde(deserialize_with = "url_deserializer")]
    pub prover_server_endpoint: Url,
    pub proving_time_ms: u64,
}

impl ProverClientConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed(PROVER_CLIENT_PREFIX)
            .from_env::<Self>()
            .map_err(ConfigError::ConfigDeserializationError)
    }

    pub fn to_env(&self) -> String {
        format!(
            "
{PROVER_CLIENT_PREFIX}_PROVER_SERVER_ENDPOINT={}
{PROVER_CLIENT_PREFIX}_PROVING_TIME_MS={}
",
            self.prover_server_endpoint, self.proving_time_ms
        )
    }

    pub fn write_env(&self) -> Result<(), TomlParserError> {
        let path = ConfigMode::ProverClient.get_env_path_or_default();
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
