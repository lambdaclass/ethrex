//! JSON-RPC transport — POST / with method/params/id envelope.

use eyre::{Context, Result};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'a str,
    method: &'a str,
    params: Value,
    id: u64,
}

#[derive(Debug)]
pub struct JsonResponse {
    pub status: u16,
    pub bytes_sent: usize,
    pub bytes_received: usize,
    // body read by Task 7 stats/output layer
    #[allow(dead_code)]
    pub body: Value,
}

pub async fn call(
    client: &Client,
    url: &str,
    token: &str,
    method: &str,
    params: Value,
) -> Result<JsonResponse> {
    let envelope = RpcRequest {
        jsonrpc: "2.0",
        method,
        params,
        id: 1,
    };
    let body = serde_json::to_vec(&envelope)?;
    let bytes_sent = body.len();
    let response = client
        .post(url)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .with_context(|| format!("JSON-RPC {method} request to {url}"))?;
    let status = response.status().as_u16();
    let raw = response.bytes().await?;
    let bytes_received = raw.len();
    let body: Value = if raw.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&raw).unwrap_or(Value::Null)
    };
    Ok(JsonResponse {
        status,
        bytes_sent,
        bytes_received,
        body,
    })
}
