use super::errors::ConfigError;
use ethereum_types::Address;
use ethrex_l2_sdk::secret_key_deserializer;
use secp256k1::SecretKey;
use serde::Deserialize;
use std::net::IpAddr;

pub const PROVER_SERVER_PREFIX: &str = "PROVER_SERVER_";

#[derive(Clone, Deserialize, Debug)]
pub struct ProverServerConfig {
    pub l1_address: Address,
    #[serde(deserialize_with = "secret_key_deserializer")]
    pub l1_private_key: SecretKey,
    pub listen_ip: IpAddr,
    pub listen_port: u16,
    pub dev_mode: bool,
}

impl ProverServerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed(PROVER_SERVER_PREFIX)
            .from_env::<Self>()
            .map_err(|e| ConfigError::ConfigDeserializationError {
                err: e,
                from: "ProverServerConfig".to_string(),
            })
    }

    pub fn to_env(&self) -> String {
        let prefix = "PROVER_SERVER";
        format!(
            "
{prefix}_L1_ADDRESS=0x{:#x}
{prefix}_L1_PRIVATE_KEY=0x{}
{prefix}_LISTEN_IP={}
{prefix}_LISTEN_PORT={}
{prefix}_DEV_MODE={}
",
            self.l1_address,
            self.l1_private_key.display_secret(),
            self.listen_ip,
            self.listen_port,
            self.dev_mode,
        )
    }
}
