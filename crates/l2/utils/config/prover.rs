use serde::Deserialize;

use super::errors::ConfigError;

#[derive(Deserialize, Debug)]
pub struct ProverConfig {
    pub prover_server_endpoint: String,
    pub proving_time_ms: u64,
    pub elasticity_multiplier: u64,
}

impl ProverConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("PROVER_CLIENT_")
            .from_env::<Self>()
            .map_err(|e| ConfigError::ConfigDeserializationError {
                err: e,
                from: "ProverWorkerConfig".to_string(),
            })
    }
}
