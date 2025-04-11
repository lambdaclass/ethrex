use ethereum_types::Address;
use serde::Deserialize;

use super::L2Config;

#[derive(Deserialize, Debug)]
pub struct BlockProducerConfig {
    pub block_time_ms: u64,
    pub coinbase_address: Address,
}

impl L2Config for BlockProducerConfig {
    const PREFIX: &str = "PROPOSER_";

    fn to_env(&self) -> String {
        format!(
            "
{prefix}BLOCK_TIME_MS={}
{prefix}COINBASE_ADDRESS={}
",
            self.block_time_ms,
            self.coinbase_address,
            prefix = Self::PREFIX,
        )
    }
}
