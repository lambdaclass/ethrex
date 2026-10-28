//! Proof engines: what verifies an execution chain proof against its public input root.
//!
//! The HTTP engine follows the proof-engine sidecar contract of Lighthouse's EIP-8025
//! integration (`POST /v1/execution_proof_verifications`, the proof bytes as the body, the
//! public input fields as query parameters, `{"status": "VALID" | "INVALID"}` back), with
//! the EIP-8440 public input fields in place of the EIP-8025 ones. No EIP-8440 engine exists
//! yet; the parameter names are this node's assumption, kept in one place. The mock engine
//! accepts [`MockProof`]s and is for tests and devnets only.

use std::time::Duration;

use async_trait::async_trait;
use ethrex_common::H256;
use libssz::SszDecode;
use reqwest::{Client, Url};

use super::types::{ExecutionProof, MockProof, PublicInput};

/// A verifier's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The proof verifies against the public input.
    Valid,
    /// It does not.
    Invalid,
}

/// The engine could not answer.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// Transport failure.
    #[error("proof engine request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// The engine answered with an error status.
    #[error("proof engine answered {0}")]
    Status(u16),
    /// The engine's answer did not parse.
    #[error("proof engine returned an unexpected body: {0}")]
    Body(String),
}

/// Verifies execution chain proofs.
#[async_trait]
pub trait ProofEngine: Send + Sync {
    /// Whether `proof` verifies against `public_input`.
    async fn verify(
        &self,
        proof: &ExecutionProof,
        public_input: &PublicInput,
    ) -> Result<Verdict, EngineError>;

    /// A short name for logs and status.
    fn name(&self) -> &'static str;
}

/// An engine reached over HTTP.
#[derive(Clone, Debug)]
pub struct HttpProofEngine {
    client: Client,
    url: Url,
}

/// Default time an engine gets to answer.
pub const DEFAULT_ENGINE_TIMEOUT: Duration = Duration::from_secs(10);

impl HttpProofEngine {
    /// An engine at `url`.
    pub fn new(url: Url, timeout: Duration) -> Result<Self, EngineError> {
        let client = Client::builder().timeout(timeout).build()?;
        Ok(Self { client, url })
    }

    fn endpoint(&self) -> Result<Url, EngineError> {
        self.url
            .join("v1/execution_proof_verifications")
            .map_err(|e| EngineError::Body(e.to_string()))
    }
}

#[derive(serde::Deserialize)]
struct VerificationResponse {
    status: String,
}

#[async_trait]
impl ProofEngine for HttpProofEngine {
    async fn verify(
        &self,
        proof: &ExecutionProof,
        public_input: &PublicInput,
    ) -> Result<Verdict, EngineError> {
        // The sidecar rebuilds the public input from its fields, as Lighthouse's EIP-8025
        // client sends them, rather than taking its root.
        let response = self
            .client
            .post(self.endpoint()?)
            .query(&[
                (
                    "origin_block_root",
                    format!("{:#x}", H256(public_input.origin_block_root)),
                ),
                (
                    "head_block_root",
                    format!("{:#x}", H256(public_input.head_block_root)),
                ),
                ("chain_id", public_input.chain_id.to_string()),
                ("schema_id", public_input.schema_id.to_string()),
                ("proof_type", proof.proof_type.to_string()),
            ])
            .header("content-type", "application/octet-stream")
            .body(proof.proof_data.as_ref().to_vec())
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(EngineError::Status(status.as_u16()));
        }
        let body: VerificationResponse = response
            .json()
            .await
            .map_err(|e| EngineError::Body(e.to_string()))?;
        match body.status.as_str() {
            "VALID" => Ok(Verdict::Valid),
            "INVALID" => Ok(Verdict::Invalid),
            other => Err(EngineError::Body(format!("status {other:?}"))),
        }
    }

    fn name(&self) -> &'static str {
        "http"
    }
}

/// Accepts a [`MockProof`] whose public values are the expected public input. Never for
/// production: it proves nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct MockProofEngine;

#[async_trait]
impl ProofEngine for MockProofEngine {
    async fn verify(
        &self,
        proof: &ExecutionProof,
        public_input: &PublicInput,
    ) -> Result<Verdict, EngineError> {
        let Some(mock) = MockProof::decode(proof.proof_data.as_ref()) else {
            return Ok(Verdict::Invalid);
        };
        let Ok(claimed) = PublicInput::from_ssz_bytes(mock.public_values.as_ref()) else {
            return Ok(Verdict::Invalid);
        };
        Ok(if claimed == *public_input {
            Verdict::Valid
        } else {
            Verdict::Invalid
        })
    }

    fn name(&self) -> &'static str {
        "mock"
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// A sidecar that records one verification request and answers `status`.
    async fn sidecar(
        status: &'static str,
    ) -> (
        std::net::SocketAddr,
        std::sync::Arc<std::sync::Mutex<Option<(Vec<(String, String)>, String, Vec<u8>)>>>,
    ) {
        use axum::{Router, extract::Query, http::HeaderMap, routing::post};
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let recorder = std::sync::Arc::clone(&seen);
        let app = Router::new().route(
            "/v1/execution_proof_verifications",
            post(
                move |Query(query): Query<Vec<(String, String)>>,
                      headers: HeaderMap,
                      body: axum::body::Bytes| {
                    let recorder = std::sync::Arc::clone(&recorder);
                    async move {
                        let content_type = headers
                            .get("content-type")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default()
                            .to_string();
                        *recorder.lock().unwrap() = Some((query, content_type, body.to_vec()));
                        axum::Json(serde_json::json!({ "status": status }))
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (addr, seen)
    }

    #[tokio::test]
    async fn http_engine_speaks_the_sidecar_contract() {
        let input = PublicInput::new(H256::repeat_byte(1), H256::repeat_byte(2), 11155111);
        let proof = ExecutionProof {
            proof_data: vec![9, 8, 7].try_into().unwrap(),
            proof_type: 2,
            origin_block_root: input.origin_block_root,
            head_block_root: input.head_block_root,
        };

        let (addr, seen) = sidecar("VALID").await;
        let engine = HttpProofEngine::new(
            format!("http://{addr}/").parse().unwrap(),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(engine.verify(&proof, &input).await.unwrap(), Verdict::Valid);
        let (query, content_type, body) = seen.lock().unwrap().clone().unwrap();
        assert_eq!(
            query,
            vec![
                (
                    "origin_block_root".to_string(),
                    format!("{:#x}", H256::repeat_byte(1))
                ),
                (
                    "head_block_root".to_string(),
                    format!("{:#x}", H256::repeat_byte(2))
                ),
                ("chain_id".to_string(), "11155111".to_string()),
                ("schema_id".to_string(), "5377".to_string()),
                ("proof_type".to_string(), "2".to_string()),
            ]
        );
        assert_eq!(content_type, "application/octet-stream");
        assert_eq!(body, vec![9, 8, 7]);

        let (addr, _) = sidecar("INVALID").await;
        let engine = HttpProofEngine::new(
            format!("http://{addr}/").parse().unwrap(),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(
            engine.verify(&proof, &input).await.unwrap(),
            Verdict::Invalid
        );
    }

    #[tokio::test]
    async fn mock_engine_accepts_only_matching_public_values() {
        let input = PublicInput::new(H256::repeat_byte(1), H256::repeat_byte(2), 1);
        let mock = MockProof::claiming(&input).unwrap();
        let proof = ExecutionProof {
            proof_data: mock.encode().try_into().unwrap(),
            proof_type: 3,
            origin_block_root: input.origin_block_root,
            head_block_root: input.head_block_root,
        };
        let engine = MockProofEngine;
        assert_eq!(engine.verify(&proof, &input).await.unwrap(), Verdict::Valid);
        let other = PublicInput::new(H256::repeat_byte(1), H256::repeat_byte(3), 1);
        assert_eq!(
            engine.verify(&proof, &other).await.unwrap(),
            Verdict::Invalid
        );
        let garbage = ExecutionProof {
            proof_data: vec![1, 2, 3].try_into().unwrap(),
            ..proof
        };
        assert_eq!(
            engine.verify(&garbage, &input).await.unwrap(),
            Verdict::Invalid
        );
    }
}
