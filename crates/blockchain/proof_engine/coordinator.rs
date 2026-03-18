//! L1 ProofCoordinator GenServer — manages prover connections and proof delivery.
//!
//! Follows the same `spawned_concurrency::GenServer` pattern as the L2
//! `ProofCoordinator`, using the generic `ProofData<ProgramInput>` protocol
//! from `ethrex-prover` for communication with prover workers.
//!
//! - Accepts TCP connections from prover workers
//! - Serves `ProgramInput` for pending blocks
//! - Receives completed proofs, stores them, and delivers via callback

use bytes::Bytes;
use ethrex_common::H256;
use ethrex_guest_program::input::ProgramInput;
use ethrex_l2_common::prover::{BatchProof, ProofFormat};
use ethrex_prover::ProofData;
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
use super::types::{ExecutionProofV1, GeneratedProof, MAX_PROOF_SIZE, ProofGenId, PublicInputV1};

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
pub struct PendingInput {
    pub proof_gen_id: ProofGenId,
    pub new_payload_request_root: H256,
    pub program_input: ProgramInput,
    /// Proof types requested by the beacon via `engine_requestProofsV1`.
    pub requested_proof_types: Vec<u64>,
}

/// Cast messages for the L1ProofCoordinator.
#[derive(Clone)]
pub enum CoordCastMsg {
    /// Start accepting TCP connections.
    Listen { listener: Arc<TcpListener> },
}

/// Output message (unused — coordinator runs indefinitely).
#[derive(Clone, PartialEq)]
pub enum CoordOutMsg {
    Done,
}

/// Shared pending input map, accessible from both the coordinator accept loop
/// and the ProofEngine (which inserts new inputs).
pub type PendingInputMap = Arc<std::sync::Mutex<HashMap<u64, PendingInput>>>;

/// L1 Proof Coordinator GenServer.
///
/// Manages a map of pending proof inputs keyed by block number, accepts
/// prover TCP connections, and dispatches work using the `ProofData<ProgramInput>`
/// protocol.
#[derive(Clone)]
pub struct L1ProofCoordinator {
    store: Store,
    config: ProofEngineConfig,
    /// Pending inputs awaiting proof generation: block_number → input.
    pending: PendingInputMap,
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

    /// Get a reference to the shared pending input map.
    /// This allows the ProofEngine to insert new inputs directly
    /// without going through the GenServer (which is blocked in the accept loop).
    pub fn pending_map(&self) -> PendingInputMap {
        Arc::clone(&self.pending)
    }

    /// Accept loop — runs forever accepting TCP connections from provers.
    async fn handle_listens(&self, listener: Arc<TcpListener>) {
        let addr = self.config.coordinator_addr.clone();
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

    /// Handle a witness request from a prover: return the next pending input.
    async fn handle_request(&self, stream: &mut TcpStream) -> Result<(), L1CoordinatorError> {
        debug!("Witness request received from prover");

        // Find the oldest pending input.
        let input = {
            let pending = self
                .pending
                .lock()
                .map_err(|_| L1CoordinatorError::Internal("Pending lock poisoned".to_string()))?;
            pending
                .iter()
                .min_by_key(|(bn, _)| **bn)
                .map(|(bn, input)| (*bn, input.clone()))
        };

        let response: ProofData<ProgramInput> = match input {
            Some((block_number, pending_input)) => {
                info!(block_number, "Sending witness to prover");
                ProofData::batch_response(
                    block_number,
                    pending_input.program_input,
                    ProofFormat::Compressed,
                )
            }
            None => {
                debug!("No pending witnesses for prover");
                ProofData::empty_batch_response()
            }
        };

        send_proof_data(stream, &response).await?;
        Ok(())
    }

    /// Handle a proof submission from a prover: store and ACK.
    async fn handle_submit(
        &self,
        stream: &mut TcpStream,
        batch_number: u64,
        batch_proof: &BatchProof,
    ) -> Result<(), L1CoordinatorError> {
        let prover_reported_type = batch_proof.prover_type() as u64;
        info!(
            block_number = batch_number,
            prover_reported_type, "Proof received from prover"
        );

        let proof_data = match batch_proof {
            BatchProof::ProofBytes(p) => p.proof.clone(),
            BatchProof::ProofCalldata(_) => {
                // ProofCalldata is for L2 on-chain verification; for L1 we store dummy bytes.
                vec![0xDE, 0xAD]
            }
        };

        // Validate size.
        if proof_data.len() > MAX_PROOF_SIZE {
            warn!(
                batch_number,
                size = proof_data.len(),
                "Proof exceeds MAX_PROOF_SIZE, rejecting"
            );
            let ack: ProofData<ProgramInput> = ProofData::proof_submit_ack(batch_number);
            send_proof_data(stream, &ack).await?;
            return Ok(());
        }

        // Look up the root, proof_gen_id and requested proof types from the pending map.
        let (root, proof_type, proof_gen_id) = {
            let pending = self
                .pending
                .lock()
                .map_err(|_| L1CoordinatorError::Internal("Pending lock poisoned".to_string()))?;
            match pending.get(&batch_number) {
                Some(p) => {
                    // Use the first requested proof type from the beacon's request.
                    // Fall back to the prover's self-reported type if none was requested.
                    let pt = p
                        .requested_proof_types
                        .first()
                        .copied()
                        .unwrap_or(prover_reported_type);
                    (p.new_payload_request_root, pt, p.proof_gen_id)
                }
                None => {
                    warn!(
                        block_number = batch_number,
                        "No pending input found for proof; using defaults"
                    );
                    (H256::default(), prover_reported_type, [0u8; 8])
                }
            }
        };

        // Store the proof.
        self.store
            .store_execution_proof(batch_number, root, proof_type, proof_data.clone())?;

        info!(
            block_number = batch_number,
            proof_type, "Execution proof stored"
        );

        // Remove from pending.
        match self.pending.lock() {
            Ok(mut pending) => {
                pending.remove(&batch_number);
            }
            Err(e) => {
                error!(block_number = batch_number, error = %e, "Pending lock poisoned on remove");
            }
        }

        // Deliver via callback if configured.
        if let Some(callback_url) = &self.config.callback_url {
            let generated_proof = GeneratedProof {
                proof_gen_id: Bytes::copy_from_slice(&proof_gen_id),
                execution_proof: ExecutionProofV1 {
                    proof_data: Bytes::from(proof_data),
                    proof_type,
                    public_input: PublicInputV1 {
                        new_payload_request_root: root,
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
                    info!(
                        block_number = batch_number,
                        "Proof delivered to callback URL"
                    );
                }
                Ok(resp) => {
                    warn!(
                        block_number = batch_number,
                        status = %resp.status(),
                        "Callback delivery returned non-success status"
                    );
                }
                Err(e) => {
                    error!(block_number = batch_number, error = %e, "Failed to deliver proof via callback");
                }
            }
        }

        // ACK.
        let ack: ProofData<ProgramInput> = ProofData::proof_submit_ack(batch_number);
        send_proof_data(stream, &ack).await?;
        info!(block_number = batch_number, "Proof ACK sent to prover");

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

            // Parse as ProofData<ProgramInput>.
            let msg: Result<ProofData<ProgramInput>, _> = serde_json::from_slice(&buffer);
            match msg {
                Ok(proof_data) => match proof_data {
                    ProofData::BatchRequest { .. } => {
                        if let Err(e) = self.coordinator.handle_request(&mut stream).await {
                            error!("Failed to handle batch request: {e}");
                        }
                    }
                    ProofData::ProofSubmit {
                        batch_number,
                        batch_proof,
                    } => {
                        if let Err(e) = self
                            .coordinator
                            .handle_submit(&mut stream, batch_number, &batch_proof)
                            .await
                        {
                            error!("Failed to handle proof submit: {e}");
                        }
                    }
                    other => {
                        warn!(
                            "Unexpected ProofData variant from prover: {}",
                            std::any::type_name_of_val(&other)
                        );
                    }
                },
                Err(e) => {
                    warn!("Failed to parse prover message as ProofData: {e}");
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

/// Helper: serialize and send a `ProofData` response over TCP.
async fn send_proof_data<I: serde::Serialize>(
    stream: &mut TcpStream,
    response: &ProofData<I>,
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

    // Share the pending map with the engine (engine inserts, coordinator reads).
    let pending_map = coordinator.pending_map();
    engine.set_pending_map(pending_map);

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

    Ok(engine)
}
