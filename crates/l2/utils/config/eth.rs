use serde::Deserialize;

use super::errors::ConfigError;

#[derive(Deserialize, Debug)]
pub struct EthConfig {
    pub rpc_url: String,
}

impl EthConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("ETH_")
            .from_env::<Self>()
            .map_err(ConfigError::from)
    }
}
