use crate::utils::secret_key_deserializer;
use ethereum_types::Address;
use libsecp256k1::SecretKey;
use serde::Deserialize;

use super::errors::ConfigError;

#[derive(Deserialize)]
pub struct ProposerConfig {
    pub on_chain_proposer_address: Address,
    pub l1_address: Address,
    #[serde(deserialize_with = "secret_key_deserializer")]
    pub l1_private_key: SecretKey,
    pub interval_ms: u64,
}

impl ProposerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("PROPOSER_")
            .from_env::<Self>()
            .map_err(ConfigError::from)
    }
}
