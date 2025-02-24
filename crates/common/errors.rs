#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Error deserializing config from env: {0}")]
    ConfigDeserializationError(#[from] std::io::Error),
}
