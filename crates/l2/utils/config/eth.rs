use reqwest::Url;
use serde::Deserialize;

use super::super::parse::deserialize_optional_url;
use super::errors::ConfigError;

#[derive(Deserialize, Debug, Clone)]
pub struct EthConfig {
    pub rpc_url: String,
    #[serde(deserialize_with = "deserialize_optional_url")]
    pub remote_signer_url: Option<Url>,
    pub maximum_allowed_max_fee_per_gas: u64,
    pub maximum_allowed_max_fee_per_blob_gas: u64,
}

impl EthConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("ETH_").from_env::<Self>().map_err(|e| {
            ConfigError::ConfigDeserializationError {
                err: e,
                from: "EthConfig".to_string(),
            }
        })
    }
}
