use ethereum_types::Address;
use ethrex_l2_sdk::secret_key_deserializer;
use secp256k1::SecretKey;
use serde::Deserialize;

use super::errors::ConfigError;

pub const COMMITTER_PREFIX: &str = "COMMITTER_";

#[derive(Deserialize, Debug)]
pub struct CommitterConfig {
    pub on_chain_proposer_address: Address,
    pub l1_address: Address,
    #[serde(deserialize_with = "secret_key_deserializer")]
    pub l1_private_key: SecretKey,
    pub commit_time_ms: u64,
    pub arbitrary_base_blob_gas_price: u64,
}

impl CommitterConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed(COMMITTER_PREFIX)
            .from_env::<Self>()
            .map_err(|e| ConfigError::ConfigDeserializationError {
                err: e,
                from: "CommitterConfig".to_string(),
            })
    }

    pub fn to_env(&self) -> String {
        format!(
            "
{COMMITTER_PREFIX}ON_CHAIN_PROPOSER_ADDRESS=0x{:#x}
{COMMITTER_PREFIX}L1_ADDRESS=0x{:#x}
{COMMITTER_PREFIX}L1_PRIVATE_KEY=0x{}
{COMMITTER_PREFIX}COMMIT_TIME_MS={}
{COMMITTER_PREFIX}ARBITRARY_BASE_BLOB_GAS_PRICE={}
",
            self.on_chain_proposer_address,
            self.l1_address,
            self.l1_private_key.display_secret(),
            self.commit_time_ms,
            self.arbitrary_base_blob_gas_price,
        )
    }
}
