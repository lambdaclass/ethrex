use super::errors::ConfigError;
use crate::utils::secret_key_deserializer;
use ethereum_types::Address;
use secp256k1::SecretKey;
use serde::Deserialize;
use std::net::IpAddr;

#[derive(Clone, Deserialize)]
pub struct ProverServerConfig {
    pub listen_ip: IpAddr,
    pub listen_port: u16,
    pub verifier_address: Address,
    #[serde(deserialize_with = "secret_key_deserializer")]
    pub verifier_private_key: SecretKey,
    pub dev_mode: bool,
}

impl ProverServerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("PROVER_SERVER_")
            .from_env::<Self>()
            .map_err(ConfigError::from)
    }
}
