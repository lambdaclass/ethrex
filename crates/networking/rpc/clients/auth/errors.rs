#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Error deserializing config from env: {0}")]
    ConfigDeserializationError(#[from] envy::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum EngineClientError {
    #[error("Error sending request {0}")]
    RequestError(String),
    #[error("reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    FailedDuringExchangeCapabilities(#[from] ExchangeCapabilitiesError),
    #[error("{0}")]
    FailedDuringForkChoiceUpdate(#[from] ForkChoiceUpdateError),
    #[error("{0}")]
    FailedDuringGetPayload(#[from] GetPayloadError),
    #[error("{0}")]
    FailedDuringNewPayload(#[from] NewPayloadError),
    #[error("EngineClient failed to prepare JWT: {0}")]
    FailedToGetSystemTime(#[from] std::time::SystemTimeError),
    #[error("EngineClient failed to decode JWT secret: {0}")]
    FailedToDecodeJWTSecret(String),
    #[error("EngineClient failed to encode JWT: {0}")]
    FailedToEncodeJWT(#[from] jsonwebtoken::errors::Error),
    #[error("EngineClient failed read secret: {0}")]
    FailedToReadSecret(#[from] std::io::Error),
    #[error("EngineClient failed to serialize request body: {0}")]
    FailedToSerializeRequestBody(String),
    #[error("EngineClient System Failed after: {0}")]
    SystemFailed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ExchangeCapabilitiesError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[derive(Debug, thiserror::Error)]
pub enum ForkChoiceUpdateError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
    #[error("{0}")]
    ConversionError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GetPayloadError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}

#[derive(Debug, thiserror::Error)]
pub enum NewPayloadError {
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    SerdeJSONError(#[from] serde_json::Error),
    #[error("{0}")]
    RPCError(String),
    #[error("{0}")]
    ParseIntError(#[from] std::num::ParseIntError),
}
