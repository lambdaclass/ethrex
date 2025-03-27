use ethereum_types::Address;
use serde::Deserialize;

use super::errors::ConfigError;

#[derive(Deserialize)]
pub struct BlockProducerConfig {
    pub block_time_ms: u64,
    pub coinbase_address: Address,
}

impl BlockProducerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("PROPOSER_")
            .from_env::<Self>()
            .map_err(ConfigError::from)
    }
}
