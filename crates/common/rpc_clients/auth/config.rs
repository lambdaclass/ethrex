use crate::auth::errors::ConfigError;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct EngineApiConfig {
    pub rpc_url: String,
    pub jwt_path: String,
}

impl EngineApiConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        envy::prefixed("ENGINE_API_")
            .from_env::<Self>()
            .map_err(ConfigError::from)
    }
}
