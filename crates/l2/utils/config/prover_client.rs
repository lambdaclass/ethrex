use crate::utils::parse::url_deserializer;
use reqwest::Url;
use serde::Deserialize;

use super::errors::ConfigError;

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
            .map_err(|e| ConfigError::ConfigDeserializationError {
                err: e,
                from: "ProverClientConfig".to_string(),
            })
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
}
