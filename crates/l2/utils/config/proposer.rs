use ethereum_types::Address;
use serde::Deserialize;

use super::errors::ConfigError;

#[derive(Deserialize)]
pub struct ProposerConfig {
    pub interval_ms: u64,
    pub coinbase_address: Address,
}

impl ProposerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("PROPOSER_")
            .from_env::<Self>()
            .map_err(ConfigError::from)
    }
}
