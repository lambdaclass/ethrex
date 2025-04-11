use ethereum_types::Address;
use ethrex_l2_sdk::secret_key_deserializer;
use secp256k1::SecretKey;
use serde::Deserialize;

use super::L2Config;

#[derive(Deserialize, Debug)]
pub struct CommitterConfig {
    pub on_chain_proposer_address: Address,
    pub l1_address: Address,
    #[serde(deserialize_with = "secret_key_deserializer")]
    pub l1_private_key: SecretKey,
    pub commit_time_ms: u64,
    pub arbitrary_base_blob_gas_price: u64,
}

impl L2Config for CommitterConfig {
    const PREFIX: &str = "COMMITTER_";

    fn to_env(&self) -> String {
        format!(
            "
{prefix}ON_CHAIN_PROPOSER_ADDRESS=0x{:#x}
{prefix}L1_ADDRESS=0x{:#x}
{prefix}L1_PRIVATE_KEY=0x{}
{prefix}COMMIT_TIME_MS={}
{prefix}ARBITRARY_BASE_BLOB_GAS_PRICE={}
",
            self.on_chain_proposer_address,
            self.l1_address,
            self.l1_private_key.display_secret(),
            self.commit_time_ms,
            self.arbitrary_base_blob_gas_price,
            prefix = Self::PREFIX
        )
    }
}
