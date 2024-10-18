use serde::Deserialize;

use super::errors::ConfigError;

#[derive(Deserialize)]
pub struct ProverClientConfig {
    pub elf_path: String,
    pub prover_server_endpoint: String,
}

impl ProverClientConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("PROVER_")
            .from_env::<Self>()
            .map_err(ConfigError::from)
    }
}
