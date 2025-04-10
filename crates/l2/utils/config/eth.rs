use crate::utils::parse::url_deserializer;
use reqwest::Url;
use serde::Deserialize;

use super::errors::ConfigError;

#[derive(Deserialize, Debug)]
pub struct EthConfig {
    #[serde(deserialize_with = "url_deserializer")]
    pub rpc_url: Url,
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
