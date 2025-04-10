use crate::utils::parse::url_deserializer;
use reqwest::Url;
use serde::Deserialize;

use super::errors::ConfigError;

pub const ETH_PREFIX: &str = "ETH_";

#[derive(Deserialize, Debug)]
pub struct EthConfig {
    #[serde(deserialize_with = "url_deserializer")]
    pub rpc_url: Url,
}

impl EthConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed(ETH_PREFIX).from_env::<Self>().map_err(|e| {
            ConfigError::ConfigDeserializationError {
                err: e,
                from: "EthConfig".to_string(),
            }
        })
    }

    pub fn to_env(&self) -> String {
        format!("{ETH_PREFIX}RPC_URL={}", self.rpc_url)
    }
}
