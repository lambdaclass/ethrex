use serde::Deserialize;

use super::errors::ConfigError;

#[derive(Deserialize, Debug, Clone)]
pub struct EthConfig {
    pub rpc_url: String,
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
