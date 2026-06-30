//! REST/SSZ transport — `/engine/v1` URLs with an SSZ body. The fork for
//! fork-scoped endpoints travels in the `Eth-Execution-Version` header.

use eyre::{Context, Result};
use reqwest::{Client, Method};

#[derive(Debug)]
pub struct SszResponse {
    pub status: u16,
    pub bytes_sent: usize,
    pub bytes_received: usize,
    /// Raw response body. Decoded by callers after the timed window (hit
    /// counting only).
    pub body: bytes::Bytes,
}

pub async fn call(
    client: &Client,
    method: Method,
    url: &str,
    token: &str,
    // `Eth-Execution-Version` fork value for fork-scoped endpoints (payloads,
    // forkchoice, bodies). `None` for unscoped endpoints (blobs).
    fork: Option<&str>,
    body: Vec<u8>,
) -> Result<SszResponse> {
    let bytes_sent = body.len();
    let mut req = client
        .request(method.clone(), url)
        .header("authorization", format!("Bearer {token}"));
    if let Some(fork) = fork {
        req = req.header("eth-execution-version", fork);
    }
    if !body.is_empty() {
        req = req
            .header("content-type", "application/octet-stream")
            .body(body);
    }
    let response = req
        .send()
        .await
        .with_context(|| format!("REST/SSZ {method} request to {url}"))?;
    let status = response.status().as_u16();
    let body_bytes = response.bytes().await?;
    Ok(SszResponse {
        status,
        bytes_sent,
        bytes_received: body_bytes.len(),
        body: body_bytes,
    })
}
