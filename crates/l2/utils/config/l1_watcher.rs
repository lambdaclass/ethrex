use ethereum_types::{Address, U256};
use ethrex_l2_sdk::secret_key_deserializer;
use secp256k1::SecretKey;
use serde::Deserialize;

use super::errors::ConfigError;

#[derive(Deserialize)]
pub struct L1WatcherConfig {
    pub bridge_address: Address,
    pub check_interval_ms: u64,
    pub max_block_step: U256,
    #[serde(deserialize_with = "secret_key_deserializer")]
    pub l2_proposer_private_key: SecretKey,
}

impl L1WatcherConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("L1_WATCHER_")
            .from_env::<Self>()
            .map_err(|e| ConfigError::ConfigDeserializationError {
                err: e,
                from: "L1WatcherConfig".to_string(),
            })
    }
}
