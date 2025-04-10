use crate::utils::parse::url_deserializer;
use reqwest::Url;
use serde::Deserialize;

use super::errors::ConfigError;

#[derive(Deserialize, Debug)]
pub struct ProverClientConfig {
    #[serde(deserialize_with = "url_deserializer")]
    pub prover_server_endpoint: Url,
    pub proving_time_ms: u64,
}

impl ProverClientConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("PROVER_CLIENT_")
            .from_env::<Self>()
            .map_err(|e| ConfigError::ConfigDeserializationError {
                err: e,
                from: "ProverClientConfig".to_string(),
            })
    }
}
