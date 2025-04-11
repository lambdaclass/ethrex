use ethereum_types::{Address, U256};
use ethrex_l2_sdk::secret_key_deserializer;
use secp256k1::SecretKey;
use serde::Deserialize;

use super::L2Config;

#[derive(Deserialize, Debug)]
pub struct L1WatcherConfig {
    pub bridge_address: Address,
    pub check_interval_ms: u64,
    pub max_block_step: U256,
    #[serde(deserialize_with = "secret_key_deserializer")]
    pub l2_proposer_private_key: SecretKey,
}

impl L2Config for L1WatcherConfig {
    const PREFIX: &str = "L1_WATCHER_";

    fn to_env(&self) -> String {
        format!(
            "
{prefix}BRIDGE_ADDRESS=0x{:#x}
{prefix}CHECK_INTERVAL_MS={}
{prefix}MAX_BLOCK_STEP={}
{prefix}L2_PROPOSER_PRIVATE_KEY=0x{}
",
            self.bridge_address,
            self.check_interval_ms,
            self.max_block_step,
            self.l2_proposer_private_key.display_secret(),
            prefix = Self::PREFIX
        )
    }
}
