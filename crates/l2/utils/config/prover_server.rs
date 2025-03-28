use super::errors::ConfigError;
use ethereum_types::Address;
use ethrex_l2_sdk::secret_key_deserializer;
use secp256k1::SecretKey;
use serde::Deserialize;
use std::net::IpAddr;

#[derive(Clone, Deserialize)]
pub struct ProverServerConfig {
    pub l1_address: Address,
    #[serde(deserialize_with = "secret_key_deserializer")]
    pub l1_private_key: SecretKey,
    pub listen_ip: IpAddr,
    pub listen_port: u16,
    pub dev_mode: bool,
    pub dev_interval_ms: u64,
}

impl ProverServerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("PROVER_SERVER_")
            .from_env::<Self>()
            .map_err(|e| ConfigError::ConfigDeserializationError {
                err: e,
                from: "ProverServerConfig".to_string(),
            })
    }
}
