//! JSON-RPC transport — POST / with method/params/id envelope.

use eyre::{Context, Result};
use reqwest::Client;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct RpcRequest<'a, P> {
    jsonrpc: &'a str,
    method: &'a str,
    params: P,
    id: u64,
}

#[derive(Debug)]
pub struct JsonResponse {
    pub status: u16,
    pub bytes_sent: usize,
    pub bytes_received: usize,
    /// Raw response body. Parsed lazily by callers that need it (hit counting,
    /// run setup) so response decoding stays out of the timed window.
    pub raw: bytes::Bytes,
}

impl JsonResponse {
    /// Parse the raw body as JSON. Never called inside a timed window.
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.raw).ok()
    }
}

/// Encode the envelope and POST it. Serialization happens in here so a caller
/// timing this call covers typed-struct → wire-bytes for JSON exactly like
/// `to_ssz()` does for SSZ. Tuples serialize as JSON arrays, so pass params as
/// e.g. `(&payload, hashes, root)`.
pub async fn call<P: Serialize>(
    client: &Client,
    url: &str,
    token: &str,
    method: &str,
    params: P,
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
    Ok(JsonResponse {
        status,
        bytes_sent,
        bytes_received: raw.len(),
        raw,
    })
}
