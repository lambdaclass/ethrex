use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn set_endpoint(&mut self, endpoint: String) {
        self.endpoint = endpoint;
    }

    pub async fn send_request(&self, method: &str, params: Vec<Value>) -> Result<Value, RpcError> {
        let id = self.request_id.fetch_add(1, Ordering::Relaxed);

        let request_body = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        });

        let response = self
            .client
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
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
}
