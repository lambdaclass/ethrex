use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::Client;
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RpcError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("JSON-RPC error (code {code}): {message}")]
    JsonRpc { code: i64, message: String },
    #[error("parse error: {0}")]
    Parse(String),
}

pub struct RpcClient {
    endpoint: String,
    client: Client,
    request_id: AtomicU64,
    jwt_secret: Option<Vec<u8>>,
}

impl RpcClient {
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to build HTTP client"),
            request_id: AtomicU64::new(1),
            jwt_secret: None,
        }
    }

    pub fn new_with_jwt(endpoint: String, jwt_secret: Vec<u8>) -> Self {
        Self {
            endpoint,
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to build HTTP client"),
            request_id: AtomicU64::new(1),
            jwt_secret: Some(jwt_secret),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub async fn send_request(&self, method: &str, params: Vec<Value>) -> Result<Value, RpcError> {
        let id = self.request_id.fetch_add(1, Ordering::Relaxed);

        let request_body = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        });

        let mut builder = self
            .client
            .post(&self.endpoint)
            .header("Content-Type", "application/json");

        if let Some(secret) = &self.jwt_secret {
            let token = Self::auth_token(secret)
                .map_err(|e| RpcError::Transport(format!("JWT error: {e}")))?;
            builder = builder.bearer_auth(token);
        }

        let response = builder
            .json(&request_body)
            .send()
            .await
            .map_err(|e| RpcError::Transport(e.to_string()))?
            .error_for_status()
            .map_err(|e| RpcError::Transport(e.to_string()))?;

        let response_body: Value = response
            .json()
            .await
            .map_err(|e| RpcError::Parse(e.to_string()))?;

        if let Some(error) = response_body.get("error") {
            let code = error.get("code").and_then(Value::as_i64).unwrap_or(-1);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string();
            return Err(RpcError::JsonRpc { code, message });
        }

        response_body
            .get("result")
            .cloned()
            .ok_or_else(|| RpcError::Parse("response missing 'result' field".to_string()))
    }

    fn auth_token(secret: &[u8]) -> Result<String, String> {
        let header = jsonwebtoken::Header::default();
        let valid_iat = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        let claims = json!({"iat": valid_iat});
        let encoding_key = jsonwebtoken::EncodingKey::from_secret(secret);
        jsonwebtoken::encode(&header, &claims, &encoding_key).map_err(|e| e.to_string())
    }
}
