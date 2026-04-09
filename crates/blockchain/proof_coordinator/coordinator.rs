//! L1 ProofCoordinator actor — manages prover connections and proof delivery.
//!
//! Follows the same `spawned_concurrency` actor pattern as the L2
//! `ProofCoordinator`, using the generic `ProofData<ProgramInput>` protocol
//! from `ethrex-prover` for communication with prover workers.
//!
//! - Accepts TCP connections from prover workers
//! - Serves `ProgramInput` for pending blocks
//! - Receives completed proofs, stores them, and delivers via callback
//!
//! # Future direction
//!
//! Currently the coordinator passively waits for provers to connect and request
//! work. A future refactor may invert this so the coordinator actively pushes
//! requests to registered provers instead.

use bytes::Bytes;
use ethrex_common::H256;
use ethrex_common::types::Block;
use ethrex_common::types::block_execution_witness::ExecutionWitness;
use ethrex_guest_program::input::ProgramInput;
use ethrex_prover::ProofData;
use ethrex_prover::{ProofFormat, ProverOutput, ProverType};
use ethrex_storage::Store;
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorStart as _, Context, Handler, send_after},
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tracing::{debug, error, info, warn};

use super::config::ProofCoordinatorConfig;
use super::types::{ExecutionProofV1, GeneratedProof, MAX_PROOF_SIZE, ProofGenId, PublicInputV1};

/// How long to wait for a prover connection before re-checking the message queue.
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Maximum time to read a complete message from a connected prover.
const CONNECTION_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum message size from a prover (10 MiB). This bounds memory usage
/// per connection and prevents a malicious peer from causing OOM.
const MAX_PROVER_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

/// Maximum number of pending proof requests. At one request per block (~12s),
/// 256 entries covers ~50 minutes of unanswered requests.
const MAX_PENDING_REQUESTS: usize = 256;

/// Pending requests older than this are evicted. This prevents memory leaks
/// when a proof type has no prover available.
const PENDING_REQUEST_TTL: Duration = Duration::from_secs(30 * 60); // 30 minutes

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

/// A proof generation request created by `engine_requestProofsV1` and
/// consumed by prover workers via the coordinator.
#[derive(Clone)]
pub struct ProofRequest {
    pub proof_gen_id: ProofGenId,
    pub new_payload_request_root: H256,
    pub block: Block,
    pub witness: ExecutionWitness,
    /// Proof types requested by the beacon via `engine_requestProofsV1`.
    pub requested_proof_types: Vec<u64>,
    /// When this request was created, used for TTL-based eviction.
    pub created_at: Instant,
}

/// Protocol for the L1ProofCoordinator actor.
#[protocol]
pub trait L1CoordinatorProtocol: Send + Sync {
    /// Try to accept the next prover connection, then re-enqueue via `send_after`.
    fn accept_next(&self) -> Result<(), ActorError>;
    /// Enqueue a new proof request from the RPC handler.
    fn add_request(&self, block_number: u64, request: Box<ProofRequest>) -> Result<(), ActorError>;
}

/// Handle to send messages to the L1ProofCoordinator actor.
///
/// Used by RPC handlers to enqueue proof requests via
/// `coordinator.send(l1_coordinator_protocol::AddRequest { .. })`.
pub type CoordinatorHandle = Context<L1ProofCoordinator>;

/// L1 Proof Coordinator actor.
///
/// Manages a map of pending proof requests keyed by block number, accepts
/// prover TCP connections, and dispatches work using the `ProofData<ProgramInput>`
/// protocol.
///
/// Instead of blocking in an accept loop, the coordinator uses a self-rescheduling
/// `AcceptNext` message: each iteration attempts to accept a connection (with a
/// short timeout), handles it, then enqueues the next `AcceptNext` via `send_after`.
/// This allows `AddRequest` messages from the RPC handlers to be interleaved
/// between accept iterations.
// TODO: For production, consider persisting pending requests in the DB instead
// of holding them in memory. Currently if the node restarts, pending requests
// are lost.
#[derive(Clone)]
pub struct L1ProofCoordinator {
    store: Store,
    config: ProofCoordinatorConfig,
    /// TCP listener for prover connections.
    listener: Option<Arc<TcpListener>>,
    /// Pending requests awaiting proof generation: block_number → request.
    pending: HashMap<u64, ProofRequest>,
    /// HTTP client for callback delivery.
    http_client: reqwest::Client,
}

impl L1ProofCoordinator {
    /// Create a new L1ProofCoordinator.
    pub fn new(store: Store, config: ProofCoordinatorConfig) -> Self {
        Self {
            store,
            config,
            listener: None,
            pending: HashMap::new(),
            http_client: reqwest::Client::new(),
        }
    }

    /// Try to accept one prover connection with a short timeout.
    /// Returns `None` if no connection arrived within the timeout.
    async fn try_accept(&self) -> Option<(TcpStream, SocketAddr)> {
        let listener = self.listener.as_ref()?;
        match tokio::time::timeout(ACCEPT_POLL_INTERVAL, listener.accept()).await {
            Ok(Ok(conn)) => Some(conn),
            Ok(Err(e)) => {
                error!("Failed to accept prover connection: {e}");
                None
            }
            Err(_) => None, // Timeout — no connection pending.
        }
    }

    /// Handle one accept iteration: check for a connection, handle it,
    /// then reschedule.
    async fn do_accept_next(&mut self, ctx: &Context<Self>) {
        if let Some((stream, peer_addr)) = self.try_accept().await {
            debug!("Prover connected from {peer_addr}");
            self.handle_connection(stream).await;
        }

        // Reschedule: the next AcceptNext will be processed after any
        // AddRequest messages already in the queue.
        send_after(
            Duration::ZERO,
            ctx.clone(),
            l1_coordinator_protocol::AcceptNext,
        );
    }

    /// Handle a single prover connection with timeout and size limit.
    async fn handle_connection(&mut self, mut stream: TcpStream) {
        let mut buffer = Vec::with_capacity(4096);
        let read_result = tokio::time::timeout(CONNECTION_READ_TIMEOUT, async {
            loop {
                let mut chunk = [0u8; 8192];
                match stream.read(&mut chunk).await {
                    Ok(0) => break Ok(()),
                    Ok(n) => {
                        if buffer.len() + n > MAX_PROVER_MESSAGE_SIZE {
                            break Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!(
                                    "Message exceeds maximum size of {MAX_PROVER_MESSAGE_SIZE} bytes"
                                ),
                            ));
                        }
                        buffer.extend_from_slice(&chunk[..n]);
                    }
                    Err(e) => break Err(e),
                }
            }
        })
        .await;

        match read_result {
            Err(_) => {
                warn!("Timed out reading from prover connection");
                return;
            }
            Ok(Err(e)) => {
                error!("Failed to read from prover: {e}");
                return;
            }
            Ok(Ok(())) => {}
        }

        let msg: Result<ProofData<ProgramInput>, _> = serde_json::from_slice(&buffer);
        match msg {
            Ok(proof_data) => match proof_data {
                ProofData::InputRequest { prover_type, .. } => {
                    if let Err(e) = self.handle_request(&mut stream, prover_type).await {
                        error!("Failed to handle input request: {e}");
                    }
                }
                ProofData::ProofSubmit {
                    id: block_number,
                    ref proof,
                } => {
                    if let Err(e) = self.handle_submit(&mut stream, block_number, proof).await {
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
    }

    /// Handle a witness request from a prover: return the next pending input
    /// that still needs a proof of the given `prover_type`.
    async fn handle_request(
        &self,
        stream: &mut TcpStream,
        prover_type: ProverType,
    ) -> Result<(), L1CoordinatorError> {
        debug!(%prover_type, "Input request received from prover");

        // Find the oldest pending request that still needs this proof type.
        // Sort by block number and return the first match to avoid unnecessary DB lookups.
        let mut candidates: Vec<_> = self.pending.iter().collect();
        candidates.sort_by_key(|(bn, _)| **bn);

        let input = candidates.into_iter().find_map(|(&bn, req)| {
            let type_requested = req.requested_proof_types.is_empty()
                || req.requested_proof_types.contains(&(prover_type as u64));
            if !type_requested {
                return None;
            }
            let already_proved = self
                .store
                .get_execution_proof(bn, &req.new_payload_request_root, prover_type as u64)
                .ok()
                .flatten()
                .is_some();
            if already_proved {
                return None;
            }
            Some((bn, req.clone()))
        });

        let response: ProofData<ProgramInput> = match input {
            Some((block_number, request)) => {
                info!(block_number, %prover_type, "Sending witness to prover");
                let program_input = ProgramInput::new(vec![request.block], request.witness);
                ProofData::input_response(block_number, program_input, ProofFormat::Compressed)
            }
            None => {
                debug!(%prover_type, "No pending work for this prover type");
                ProofData::empty_input_response()
            }
        };

        send_response(stream, &response).await?;
        Ok(())
    }

    /// Handle a proof submission from a prover: store and ACK.
    async fn handle_submit(
        &mut self,
        stream: &mut TcpStream,
        block_number: u64,
        proof: &ProverOutput,
    ) -> Result<(), L1CoordinatorError> {
        let prover_reported_type = proof.prover_type() as u64;
        info!(
            block_number,
            prover_reported_type, "Proof received from prover"
        );

        let proof_data = proof.proof_bytes().proof.clone();

        // Validate size.
        if proof_data.len() > MAX_PROOF_SIZE {
            warn!(
                block_number,
                size = proof_data.len(),
                "Proof exceeds MAX_PROOF_SIZE, rejecting"
            );
            let ack: ProofData<ProgramInput> = ProofData::proof_submit_ack(block_number);
            send_response(stream, &ack).await?;
            return Ok(());
        }

        // Look up the root, proof_gen_id and requested proof types from pending.
        // The prover reports its own type — validate it against the requested types.
        // TODO: handle reorgs — if the canonical chain changes, the stored root
        // for this block_number may no longer be correct.
        let (root, proof_gen_id) = match self.pending.get(&block_number) {
            Some(p) => {
                if !p.requested_proof_types.is_empty()
                    && !p.requested_proof_types.contains(&prover_reported_type)
                {
                    warn!(
                        block_number,
                        prover_reported_type,
                        requested = ?p.requested_proof_types,
                        "Prover reported a type not in the requested set; rejecting"
                    );
                    let ack: ProofData<ProgramInput> = ProofData::proof_submit_ack(block_number);
                    send_response(stream, &ack).await?;
                    return Ok(());
                }
                (p.new_payload_request_root, p.proof_gen_id)
            }
            None => {
                warn!(
                    block_number,
                    "No pending request found for proof; rejecting submission"
                );
                let ack: ProofData<ProgramInput> = ProofData::proof_submit_ack(block_number);
                send_response(stream, &ack).await?;
                return Ok(());
            }
        };

        // Store the proof keyed by the prover's actual type.
        self.store.store_execution_proof(
            block_number,
            root,
            prover_reported_type,
            proof_data.clone(),
        )?;

        info!(
            block_number,
            proof_type = prover_reported_type,
            "Execution proof stored"
        );

        // Remove from pending once all requested proof types have been fulfilled.
        // When requested_proof_types is empty (meaning "accept any"), a single proof suffices.
        if let Some(req) = self.pending.get(&block_number) {
            let should_remove = if req.requested_proof_types.is_empty() {
                // "Accept any type" — one proof is enough.
                true
            } else {
                req.requested_proof_types.iter().all(|pt| {
                    self.store
                        .get_execution_proof(block_number, &root, *pt)
                        .ok()
                        .flatten()
                        .is_some()
                })
            };
            if should_remove {
                self.pending.remove(&block_number);
                debug!(
                    block_number,
                    "All requested proof types fulfilled; removed from pending"
                );
            }
        }

        // Deliver via callback if configured.
        // NOTE: No retry logic — if the callback fails, the proof is still stored
        // in the DB and can be queried via engine_verifyNewPayloadRequestHeaderV1.
        // A production implementation should add exponential-backoff retries.
        if let Some(callback_url) = &self.config.callback_url {
            let generated_proof = GeneratedProof {
                proof_gen_id: Bytes::copy_from_slice(&proof_gen_id),
                execution_proof: ExecutionProofV1 {
                    proof_data: Bytes::from(proof_data),
                    proof_type: prover_reported_type,
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
        let ack: ProofData<ProgramInput> = ProofData::proof_submit_ack(block_number);
        send_response(stream, &ack).await?;
        info!(block_number, "Proof ACK sent to prover");

        Ok(())
    }

    /// Remove pending requests that have exceeded the TTL.
    fn evict_stale_requests(&mut self) {
        let now = Instant::now();
        let before = self.pending.len();
        self.pending
            .retain(|_bn, req| now.duration_since(req.created_at) < PENDING_REQUEST_TTL);
        let evicted = before - self.pending.len();
        if evicted > 0 {
            info!(
                evicted,
                "Evicted stale pending proof requests (TTL expired)"
            );
        }
    }
}

#[actor(protocol = L1CoordinatorProtocol)]
impl L1ProofCoordinator {
    #[started]
    async fn started(&mut self, ctx: &Context<Self>) {
        // Kick off the accept cycle.
        let _ = ctx
            .send(l1_coordinator_protocol::AcceptNext)
            .inspect_err(|e| error!("Failed to send initial AcceptNext: {e}"));
    }

    #[send_handler]
    async fn handle_accept_next(
        &mut self,
        _msg: l1_coordinator_protocol::AcceptNext,
        ctx: &Context<Self>,
    ) {
        self.do_accept_next(ctx).await;
    }

    #[send_handler]
    async fn handle_add_request(
        &mut self,
        msg: l1_coordinator_protocol::AddRequest,
        _ctx: &Context<Self>,
    ) {
        // Evict stale entries before adding.
        self.evict_stale_requests();

        // Enforce capacity limit: evict the oldest (lowest block number) entry.
        if self.pending.len() >= MAX_PENDING_REQUESTS
            && let Some(&oldest_bn) = self.pending.keys().min()
        {
            self.pending.remove(&oldest_bn);
            warn!(
                evicted_block = oldest_bn,
                "Pending queue at capacity ({MAX_PENDING_REQUESTS}); evicted oldest request"
            );
        }

        let block_number = msg.block_number;
        self.pending.insert(block_number, *msg.request);
        debug!(block_number, "Proof request added to queue");
    }
}

/// Helper: serialize and send a `ProofData` response over TCP.
async fn send_response<I: serde::Serialize>(
    stream: &mut TcpStream,
    response: &ProofData<I>,
) -> Result<(), L1CoordinatorError> {
    let buffer = serde_json::to_vec(response)?;
    stream.write_all(&buffer).await?;
    Ok(())
}

/// Start the proof coordinator and return a handle for sending messages to it.
///
/// Call this during node startup when the `eip-8025` feature is enabled.
/// The returned `CoordinatorHandle` is used by RPC handlers to enqueue
/// new proof requests via `coordinator.send(l1_coordinator_protocol::AddRequest { .. })`.
pub async fn start_proof_coordinator(
    store: Store,
    config: ProofCoordinatorConfig,
) -> Result<CoordinatorHandle, L1CoordinatorError> {
    // Bind the TCP listener before starting the actor.
    let bind_addr = format!("{}:{}", config.coordinator_addr, config.coordinator_port);
    let listener = Arc::new(TcpListener::bind(&bind_addr).await?);
    info!("L1 ProofCoordinator bound to {bind_addr}");

    let mut coordinator = L1ProofCoordinator::new(store, config);
    coordinator.listener = Some(listener);

    // The #[started] hook sends the initial AcceptNext message automatically.
    let actor_ref = coordinator.start();

    Ok(actor_ref.context())
}
