use std::net::IpAddr;

use serde::Deserialize;

use super::errors::ConfigError;

#[derive(Clone, Deserialize)]
pub struct ProverServerConfig {
    pub listen_ip: IpAddr,
    pub listen_port: u16,
}

impl ProverServerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("PROVER_SERVER_")
            .from_env::<Self>()
            .map_err(ConfigError::from)
    }
}
