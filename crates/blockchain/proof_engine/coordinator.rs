//! L1 ProofCoordinator GenServer — manages prover connections and proof delivery.
//!
//! Follows the same `spawned_concurrency::GenServer` pattern as the L2
//! `ProofCoordinator`, but simplified for L1 EIP-8025 use:
//!
//! - Accepts TCP connections from prover workers
//! - Serves `ProgramInput` for pending blocks
//! - Receives completed proofs, stores them, and delivers via callback

use bytes::Bytes;
use ethrex_common::types::block_execution_witness::ExecutionWitness;
use ethrex_common::H256;
use ethrex_storage::Store;
use spawned_concurrency::messages::Unused;
use spawned_concurrency::tasks::{CastResponse, GenServer, GenServerHandle};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tracing::{debug, error, info, warn};

use crate::Blockchain;

use super::config::ProofEngineConfig;
use super::types::{
    ExecutionProofV1, GeneratedProof, ProofGenId, PublicInputV1, MAX_PROOF_SIZE,
};

/// Error type for L1 proof coordinator operations.
#[derive(Debug, thiserror::Error)]
pub enum L1CoordinatorError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Storage error: {0}")]
    Store(#[from] ethrex_storage::error::StoreError),
    #[error("Callback delivery failed: {0}")]
    CallbackFailed(String),
    #[error("{0}")]
    Internal(String),
}

/// Pending proof generation input.
#[derive(Clone)]
#[allow(dead_code)]
struct PendingInput {
    proof_gen_id: ProofGenId,
    new_payload_request_root: H256,
    witness: ExecutionWitness,
}

/// Cast messages for the L1ProofCoordinator.
#[derive(Clone)]
pub enum CoordCastMsg {
    /// Start accepting TCP connections.
    Listen { listener: Arc<TcpListener> },
    /// A new block needs proof generation.
    NewInput {
        block_number: u64,
        proof_gen_id: ProofGenId,
        new_payload_request_root: H256,
        witness: Box<ExecutionWitness>,
    },
}

/// Output message (unused — coordinator runs indefinitely).
#[derive(Clone, PartialEq)]
pub enum CoordOutMsg {
    Done,
}

/// L1 Proof Coordinator GenServer.
///
/// Manages a map of pending proof inputs keyed by block number, accepts
/// prover TCP connections, and dispatches work.
#[derive(Clone)]
pub struct L1ProofCoordinator {
    store: Store,
    config: ProofEngineConfig,
    /// Pending inputs awaiting proof generation: block_number → input.
    pending: Arc<std::sync::Mutex<HashMap<u64, PendingInput>>>,
    /// HTTP client for callback delivery.
    http_client: reqwest::Client,
}

impl L1ProofCoordinator {
    /// Create a new L1ProofCoordinator.
    pub fn new(store: Store, config: ProofEngineConfig) -> Self {
        Self {
            store,
            config,
            pending: Arc::new(std::sync::Mutex::new(HashMap::new())),
            http_client: reqwest::Client::new(),
        }
    }

    /// Accept loop — runs forever accepting TCP connections from provers.
    async fn handle_listens(&self, listener: Arc<TcpListener>) {
        let addr = self
            .config
            .coordinator_addr
            .clone();
        let port = self.config.coordinator_port;
        info!("L1 ProofCoordinator TCP server listening on {addr}:{port}");

        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    debug!("Prover connected from {peer_addr}");
                    let _ = L1ConnectionHandler::spawn(self.clone(), stream, peer_addr)
                        .await
                        .inspect_err(|err| {
                            error!("Error starting L1ConnectionHandler: {err}");
                        });
                }
                Err(e) => {
                    error!("Failed to accept prover connection: {e}");
                }
            }
        }
    }

    /// Handle a proof request from a prover: return the next pending input.
    async fn handle_request(
        &self,
        stream: &mut TcpStream,
    ) -> Result<(), L1CoordinatorError> {
        info!("Proof request received from prover");

        // Find the oldest pending input.
        let input = {
            let pending = self.pending.lock().map_err(|_| {
                L1CoordinatorError::Internal("Pending lock poisoned".to_string())
            })?;
            // Get the entry with the lowest block number.
            pending
                .iter()
                .min_by_key(|(bn, _)| **bn)
                .map(|(bn, input)| (*bn, input.clone()))
        };

        let response = match input {
            Some((block_number, pending_input)) => {
                serde_json::json!({
                    "type": "batch_response",
                    "block_number": block_number,
                    "proof_gen_id": hex::encode(pending_input.proof_gen_id),
                    "new_payload_request_root": pending_input.new_payload_request_root,
                    "has_input": true
                })
            }
            None => {
                serde_json::json!({
                    "type": "batch_response",
                    "has_input": false
                })
            }
        };

        send_response(stream, &response).await?;
        Ok(())
    }

    /// Handle a proof submission from a prover.
    async fn handle_submit(
        &self,
        stream: &mut TcpStream,
        block_number: u64,
        proof_type: u64,
        proof_data: Vec<u8>,
        new_payload_request_root: H256,
    ) -> Result<(), L1CoordinatorError> {
        info!(block_number, proof_type, "ProofSubmit received");

        // Validate size.
        if proof_data.len() > MAX_PROOF_SIZE {
            warn!(
                block_number,
                size = proof_data.len(),
                "Proof exceeds MAX_PROOF_SIZE, rejecting"
            );
            let ack = serde_json::json!({ "type": "proof_submit_ack", "block_number": block_number, "accepted": false });
            send_response(stream, &ack).await?;
            return Ok(());
        }

        // Store the proof.
        self.store.store_execution_proof(
            block_number,
            new_payload_request_root,
            proof_type,
            proof_data.clone(),
        )?;

        info!(block_number, proof_type, "Execution proof stored");

        // Remove from pending.
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&block_number);
        }

        // Deliver via callback if configured.
        if let Some(callback_url) = &self.config.callback_url {
            // Look up the proof_gen_id from our pending map (may have been removed).
            let proof_gen_id_bytes = {
                // We already removed it, so reconstruct from block_number + root.
                let mut id = [0u8; 8];
                id[..4].copy_from_slice(&block_number.to_be_bytes()[4..]);
                id[4..].copy_from_slice(&new_payload_request_root.as_bytes()[..4]);
                id
            };

            let generated_proof = GeneratedProof {
                proof_gen_id: Bytes::copy_from_slice(&proof_gen_id_bytes),
                execution_proof: ExecutionProofV1 {
                    proof_data: Bytes::from(proof_data),
                    proof_type,
                    public_input: PublicInputV1 {
                        new_payload_request_root,
                    },
                },
            };

            match self
                .http_client
                .post(callback_url.as_str())
                .json(&generated_proof)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    info!(block_number, "Proof delivered to callback URL");
                }
                Ok(resp) => {
                    warn!(
                        block_number,
                        status = %resp.status(),
                        "Callback delivery returned non-success status"
                    );
                }
                Err(e) => {
                    error!(block_number, error = %e, "Failed to deliver proof via callback");
                }
            }
        }

        // ACK.
        let ack = serde_json::json!({ "type": "proof_submit_ack", "block_number": block_number, "accepted": true });
        send_response(stream, &ack).await?;
        info!(block_number, "ProofSubmit ACK sent");

        Ok(())
    }
}

impl GenServer for L1ProofCoordinator {
    type CallMsg = Unused;
    type CastMsg = CoordCastMsg;
    type OutMsg = CoordOutMsg;
    type Error = L1CoordinatorError;

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            CoordCastMsg::Listen { listener } => {
                self.handle_listens(listener).await;
            }
            CoordCastMsg::NewInput {
                block_number,
                proof_gen_id,
                new_payload_request_root,
                witness,
            } => {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.insert(
                        block_number,
                        PendingInput {
                            proof_gen_id,
                            new_payload_request_root,
                            witness: *witness,
                        },
                    );
                    debug!(block_number, "Added pending input for proof generation");
                }
                return CastResponse::NoReply;
            }
        }
        CastResponse::Stop
    }
}

// ── L1ConnectionHandler ──────────────────────────────────────────────

/// Per-connection handler for prover TCP connections.
#[derive(Clone)]
struct L1ConnectionHandler {
    coordinator: L1ProofCoordinator,
}

/// Cast messages for the connection handler.
#[derive(Clone)]
enum ConnCastMsg {
    Connection {
        stream: Arc<TcpStream>,
        addr: SocketAddr,
    },
}

/// Output message (unused — required by GenServer trait).
#[derive(Clone, PartialEq)]
#[allow(dead_code)]
enum ConnOutMsg {
    Done,
}

impl L1ConnectionHandler {
    fn new(coordinator: L1ProofCoordinator) -> Self {
        Self { coordinator }
    }

    async fn spawn(
        coordinator: L1ProofCoordinator,
        stream: TcpStream,
        addr: SocketAddr,
    ) -> Result<(), L1CoordinatorError> {
        let mut handler = Self::new(coordinator).start();
        handler
            .cast(ConnCastMsg::Connection {
                stream: Arc::new(stream),
                addr,
            })
            .await
            .map_err(|e| L1CoordinatorError::Internal(e.to_string()))
    }

    async fn handle_connection(
        &mut self,
        stream: Arc<TcpStream>,
    ) -> Result<(), L1CoordinatorError> {
        let mut buffer = Vec::new();

        if let Some(mut stream) = Arc::into_inner(stream) {
            stream.read_to_end(&mut buffer).await?;

            let data: Result<serde_json::Value, _> = serde_json::from_slice(&buffer);
            match data {
                Ok(msg) => {
                    let msg_type = msg
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");

                    match msg_type {
                        "batch_request" => {
                            if let Err(e) = self.coordinator.handle_request(&mut stream).await {
                                error!("Failed to handle batch request: {e}");
                            }
                        }
                        "proof_submit" => {
                            let block_number = msg
                                .get("block_number")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let proof_type =
                                msg.get("proof_type").and_then(|v| v.as_u64()).unwrap_or(0);
                            let proof_data = msg
                                .get("proof_data")
                                .and_then(|v| v.as_str())
                                .and_then(|s| hex::decode(s).ok())
                                .unwrap_or_default();
                            let root_bytes = msg
                                .get("new_payload_request_root")
                                .and_then(|v| v.as_str())
                                .and_then(|s| {
                                    let s = s.strip_prefix("0x").unwrap_or(s);
                                    hex::decode(s).ok()
                                })
                                .unwrap_or_default();
                            let root = if root_bytes.len() == 32 {
                                H256::from_slice(&root_bytes)
                            } else {
                                H256::zero()
                            };

                            if let Err(e) = self
                                .coordinator
                                .handle_submit(
                                    &mut stream,
                                    block_number,
                                    proof_type,
                                    proof_data,
                                    root,
                                )
                                .await
                            {
                                error!("Failed to handle proof submit: {e}");
                            }
                        }
                        _ => {
                            warn!("Unknown message type: {msg_type}");
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to parse prover message: {e}");
                }
            }
        } else {
            error!("Unable to use TCP stream");
        }

        Ok(())
    }
}

impl GenServer for L1ConnectionHandler {
    type CallMsg = Unused;
    type CastMsg = ConnCastMsg;
    type OutMsg = ConnOutMsg;
    type Error = L1CoordinatorError;

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            ConnCastMsg::Connection { stream, addr } => {
                if let Err(err) = self.handle_connection(stream).await {
                    error!("Error handling prover connection from {addr}: {err}");
                } else {
                    debug!("Prover connection from {addr} handled successfully");
                }
            }
        }
        CastResponse::Stop
    }
}

/// Helper: serialize and send a JSON response over TCP.
async fn send_response(
    stream: &mut TcpStream,
    response: &serde_json::Value,
) -> Result<(), L1CoordinatorError> {
    let buffer = serde_json::to_vec(response)?;
    stream.write_all(&buffer).await?;
    Ok(())
}

/// Initialize and start the proof engine and coordinator.
///
/// Call this during node startup when the `eip-8025` feature is enabled.
pub async fn init_proof_engine(
    blockchain: Arc<Blockchain>,
    store: Store,
    config: ProofEngineConfig,
) -> Result<super::engine::ProofEngine, L1CoordinatorError> {
    let mut engine = super::engine::ProofEngine::new(blockchain, store.clone(), config.clone());

    // Start the coordinator.
    let coordinator = L1ProofCoordinator::new(store, config.clone());
    let mut coord_handle = coordinator.start();

    // Bind the TCP listener.
    let bind_addr = format!("{}:{}", config.coordinator_addr, config.coordinator_port);
    let listener = Arc::new(TcpListener::bind(&bind_addr).await?);
    info!("L1 ProofCoordinator bound to {bind_addr}");

    // Tell the coordinator to start accepting connections.
    coord_handle
        .cast(CoordCastMsg::Listen { listener })
        .await
        .map_err(|e| L1CoordinatorError::Internal(e.to_string()))?;

    // Wire the coordinator into the engine.
    engine.set_coordinator(coord_handle);

    Ok(engine)
}
