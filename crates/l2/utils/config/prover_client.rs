use serde::Deserialize;

use super::errors::ConfigError;

#[derive(Deserialize, Debug)]
pub struct ProverClientConfig {
    pub prover_server_endpoint: String,
}

impl ProverClientConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("PROVER_CLIENT_")
            .from_env::<Self>()
            .map_err(ConfigError::from)
    }
}
