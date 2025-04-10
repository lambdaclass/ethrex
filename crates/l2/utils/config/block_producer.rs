use ethereum_types::Address;
use serde::Deserialize;

use super::errors::ConfigError;

pub const BLOCK_PRODUCER_PREFIX: &str = "PROPOSER_";

#[derive(Deserialize, Debug)]
pub struct BlockProducerConfig {
    pub block_time_ms: u64,
    pub coinbase_address: Address,
}

impl BlockProducerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed(BLOCK_PRODUCER_PREFIX)
            .from_env::<Self>()
            .map_err(|e| ConfigError::ConfigDeserializationError {
                err: e,
                from: "BlockProducerConfig".to_string(),
            })
    }

    pub fn to_env(&self) -> String {
        format!(
            "
{BLOCK_PRODUCER_PREFIX}BLOCK_TIME_MS={}
{BLOCK_PRODUCER_PREFIX}COINBASE_ADDRESS={}
",
            self.block_time_ms, self.coinbase_address,
        )
    }
}
