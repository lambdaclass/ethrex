use crate::utils::secret_key_deserializer;
use ethereum_types::{Address, H256, U256};
use secp256k1::SecretKey;
use serde::Deserialize;

use super::errors::ConfigError;

#[derive(Deserialize)]
pub struct L1WatcherConfig {
    pub bridge_address: Address,
    pub topics: Vec<H256>,
    pub check_interval_ms: u64,
    pub max_block_step: U256,
    #[serde(deserialize_with = "secret_key_deserializer")]
    pub l2_proposer_private_key: SecretKey,
    pub l2_proposer_address: Address,
}

impl L1WatcherConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("L1_WATCHER_")
            .from_env::<Self>()
            .map_err(ConfigError::from)
    }
}
