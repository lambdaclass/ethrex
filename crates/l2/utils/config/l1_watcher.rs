use ethereum_types::{Address, U256};
use ethrex_l2_sdk::secret_key_deserializer;
use secp256k1::SecretKey;
use serde::Deserialize;

use super::errors::ConfigError;

pub const L1_WATCHER_PREFIX: &str = "L1_WATCHER_";

#[derive(Deserialize, Debug)]
pub struct L1WatcherConfig {
    pub bridge_address: Address,
    pub check_interval_ms: u64,
    pub max_block_step: U256,
    #[serde(deserialize_with = "secret_key_deserializer")]
    pub l2_proposer_private_key: SecretKey,
}

impl L1WatcherConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed(L1_WATCHER_PREFIX)
            .from_env::<Self>()
            .map_err(|e| ConfigError::ConfigDeserializationError {
                err: e,
                from: "L1WatcherConfig".to_string(),
            })
    }

    pub fn to_env(&self) -> String {
        format!(
            "
{L1_WATCHER_PREFIX}BRIDGE_ADDRESS=0x{:#x}
{L1_WATCHER_PREFIX}CHECK_INTERVAL_MS={}
{L1_WATCHER_PREFIX}MAX_BLOCK_STEP={}
{L1_WATCHER_PREFIX}L2_PROPOSER_PRIVATE_KEY=0x{}
",
            self.bridge_address,
            self.check_interval_ms,
            self.max_block_step,
            self.l2_proposer_private_key.display_secret()
        )
    }
}
