use crate::SequencerConfig;
use crate::sequencer::errors::{ConnectionHandlerError, ProofCoordinatorError};
use crate::sequencer::setup::{prepare_quote_prerequisites, register_tdx_key};
use crate::sequencer::utils::get_git_commit_hash;
use bytes::Bytes;
use ethrex_common::Address;
use ethrex_l2_common::prover::{ProofData, ProofFormat, ProverInputData, ProverOutput, ProverType};
use ethrex_metrics::metrics;
use ethrex_rpc::clients::eth::EthClient;
use ethrex_storage_rollup::StoreRollup;
use futures::stream;
use secp256k1::SecretKey;
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{Actor, ActorStart as _, Context, Handler, spawn_listener},
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

#[cfg(feature = "metrics")]
use ethrex_metrics::l2::metrics::METRICS;

#[protocol]
pub trait ProofCoordinatorProtocol: Send + Sync {
    fn incoming_connection(
        &self,
        stream: Arc<TcpStream>,
        addr: SocketAddr,
    ) -> Result<(), ActorError>;
}

#[derive(Clone)]
pub struct ProofCoordinator {
    eth_client: EthClient,
    on_chain_proposer_address: Address,
    rollup_store: StoreRollup,
    rpc_url: String,
    tdx_private_key: Option<SecretKey>,
    needed_proof_types: Vec<ProverType>,
    aligned: bool,
    git_commit_hash: String,
    qpl_tool_path: Option<String>,
    /// Tracks batch assignments to provers: (batch_number, prover_type) -> assignment time.
    /// In-memory only; lost on restart. Keyed per proof type so that e.g. a RISC0
    /// assignment doesn't block an SP1 prover from working on the same batch.
    assignments: Arc<std::sync::Mutex<HashMap<(u64, ProverType), Instant>>>,
    prover_timeout: Duration,
}

#[actor(protocol = ProofCoordinatorProtocol)]
impl ProofCoordinator {
    pub fn new(
        config: &SequencerConfig,
        rollup_store: StoreRollup,
        needed_proof_types: Vec<ProverType>,
    ) -> Result<Self, ProofCoordinatorError> {
        let eth_client = EthClient::new_with_config(
            config.eth.rpc_url.clone(),
            config.eth.max_number_of_retries,
            config.eth.backoff_factor,
            config.eth.min_retry_delay,
            config.eth.max_retry_delay,
            Some(config.eth.maximum_allowed_max_fee_per_gas),
            Some(config.eth.maximum_allowed_max_fee_per_blob_gas),
        )?;
        let on_chain_proposer_address = config.l1_committer.on_chain_proposer_address;

        let rpc_url = config
            .eth
            .rpc_url
            .first()
            .ok_or(ProofCoordinatorError::Custom(
                "no rpc urls present!".to_string(),
            ))?
            .to_string();

        Ok(Self {
            eth_client,
            on_chain_proposer_address,
            rollup_store,
            rpc_url,
            tdx_private_key: config.proof_coordinator.tdx_private_key,
            needed_proof_types,
            git_commit_hash: get_git_commit_hash(),
            aligned: config.aligned.aligned_mode,
            qpl_tool_path: config.proof_coordinator.qpl_tool_path.clone(),
            assignments: Arc::new(std::sync::Mutex::new(HashMap::new())),
            prover_timeout: Duration::from_millis(config.proof_coordinator.prover_timeout_ms),
        })
    }

    pub async fn spawn(
        rollup_store: StoreRollup,
        cfg: SequencerConfig,
        needed_proof_types: Vec<ProverType>,
    ) -> Result<(), ProofCoordinatorError> {
        let listen_ip = cfg.proof_coordinator.listen_ip;
        let port = cfg.proof_coordinator.listen_port;
        let state = Self::new(&cfg, rollup_store, needed_proof_types)?;
        let listener = TcpListener::bind(format!("{listen_ip}:{port}")).await?;
        let actor_ref = state.start();

        info!("Starting TCP server at {listen_ip}:{port}.");
        let accept_stream = stream::unfold(listener, |listener| async move {
            loop {
                match listener.accept().await {
                    Ok((stream, addr)) => {
                        return Some((
                            proof_coordinator_protocol::IncomingConnection {
                                stream: Arc::new(stream),
                                addr,
                            },
                            listener,
                        ));
                    }
                    Err(e) => {
                        error!("Failed to accept connection: {e}");
                    }
                }
            }
        });
        spawn_listener(actor_ref.context(), accept_stream);

        Ok(())
    }

    #[send_handler]
    async fn handle_incoming_connection(
        &mut self,
        msg: proof_coordinator_protocol::IncomingConnection,
        _ctx: &Context<Self>,
    ) {
        let _ = ConnectionHandler::spawn(self.clone(), msg.stream, msg.addr)
            .await
            .inspect_err(|err| {
                error!("Error starting ConnectionHandler: {err}");
            });
    }

    async fn next_batch_to_assign(
        &self,
        commit_hash: &str,
        prover_type: ProverType,
    ) -> Result<Option<(u64, ProverInputData)>, ProofCoordinatorError> {
        let base_batch = 1 + self.rollup_store.get_latest_verified_batch_proof().await?.0;

        loop {
            // Lock briefly to find and claim a candidate
            let candidate = {
                let mut assignments = self.assignments.lock().map_err(|_| {
                    ProofCoordinatorError::Custom("Assignment lock poisoned".to_string())
                })?;

                assignments.retain(|&(batch, _), _| batch >= base_batch);

                let now = Instant::now();
                let mut batch = base_batch;
                // Upper bound: there can be at most assignments.len() consecutive
                // assigned batches for this prover type.
                let max_batch =
                    base_batch.saturating_add(u64::try_from(assignments.len()).unwrap_or(u64::MAX));

                let key = |b| (b, prover_type);
                while batch <= max_batch {
                    match assignments.get(&key(batch)) {
                        None => break,
                        Some(&assigned_at)
                            if now.duration_since(assigned_at) > self.prover_timeout =>
                        {
                            break;
                        }
                        Some(_) => batch += 1,
                    }
                }

                assignments.insert(key(batch), now);
                batch
            };

            // No prover input for this version — nothing left to assign
            let Some(input) = self
                .rollup_store
                .get_prover_input_by_batch_and_version(candidate, commit_hash)
                .await?
            else {
                if let Ok(mut assignments) = self.assignments.lock() {
                    assignments.remove(&(candidate, prover_type));
                }
                return Ok(None);
            };

            // Skip batches where this proof type already exists (keep assignment
            // so the scan advances past it on next iteration)
            if self
                .rollup_store
                .get_proof_by_batch_and_type(candidate, prover_type)
                .await?
                .is_some()
            {
                debug!("Proof for {prover_type} already exists for batch {candidate}, skipping");
                continue;
            }

            return Ok(Some((candidate, input)));
        }
    }

    async fn handle_request(
        &self,
        stream: &mut TcpStream,
        commit_hash: String,
        prover_type: ProverType,
    ) -> Result<(), ProofCoordinatorError> {
        info!("InputRequest received from {prover_type} prover");

        // Step 1: Check if this prover's type is one of the needed proof types.
        // If not, tell the prover immediately — there's no point assigning
        // any batch to it (e.g. an SP1 prover connecting when only exec
        // proofs are needed). This is a permanent rejection.
        if !self.needed_proof_types.contains(&prover_type) {
            info!("{prover_type} proof is not needed, rejecting prover");
            let response = ProofData::ProverTypeNotNeeded { prover_type };
            send_response(stream, &response).await?;
            return Ok(());
        }

        // Step 2: Find the next unassigned batch for this prover.
        let Some((batch_to_prove, input)) =
            self.next_batch_to_assign(&commit_hash, prover_type).await?
        else {
            // Distinguish "wrong version" from "no work available" so the
            // prover client knows whether its binary is outdated.
            if commit_hash != self.git_commit_hash {
                send_response(stream, &ProofData::version_mismatch()).await?;
                info!("VersionMismatch sent");
            } else {
                send_response(stream, &ProofData::empty_input_response()).await?;
                info!("Empty InputResponse sent (no work available)");
            }
            return Ok(());
        };

        let format = if self.aligned {
            ProofFormat::Compressed
        } else {
            ProofFormat::Groth16
        };
        let response = ProofData::input_response(batch_to_prove, input, format);
        send_response(stream, &response).await?;
        info!("InputResponse sent for batch number: {batch_to_prove}");

        Ok(())
    }

    async fn handle_submit(
        &self,
        stream: &mut TcpStream,
        batch_number: u64,
        proof_bytes: ProverOutput,
    ) -> Result<(), ProofCoordinatorError> {
        info!("ProofSubmit received for batch number: {batch_number}");

        // Check if we have a proof for this batch and prover type
        let prover_type = proof_bytes.prover_type();
        if self
            .rollup_store
            .get_proof_by_batch_and_type(batch_number, prover_type)
            .await?
            .is_some()
        {
            info!(
                ?batch_number,
                ?prover_type,
                "A proof was received for a batch and type that is already stored"
            );
        } else {
            metrics!(if let Ok(assignments) = self.assignments.lock()
                && let Some(&assigned_at) = assignments.get(&(batch_number, prover_type))
            {
                let proving_time: i64 =
                    assigned_at.elapsed().as_secs().try_into().map_err(|_| {
                        ProofCoordinatorError::InternalError(
                            "failed to convert proving time to i64".to_string(),
                        )
                    })?;
                METRICS.set_batch_proving_time(batch_number, proving_time)?;
            });
            self.rollup_store
                .store_proof_by_batch_and_type(batch_number, prover_type, proof_bytes)
                .await?;
        }

        // Remove the assignment for this (batch, prover_type)
        if let Ok(mut assignments) = self.assignments.lock() {
            assignments.remove(&(batch_number, prover_type));
        }

        let response = ProofData::proof_submit_ack(batch_number);
        send_response(stream, &response).await?;
        info!("ProofSubmit ACK sent");
        Ok(())
    }

    async fn handle_setup(
        &self,
        stream: &mut TcpStream,
        prover_type: ProverType,
        payload: Bytes,
    ) -> Result<(), ProofCoordinatorError> {
        info!("ProverSetup received for {prover_type}");

        match prover_type {
            ProverType::TDX => {
                let Some(key) = self.tdx_private_key.as_ref() else {
                    return Err(ProofCoordinatorError::MissingTDXPrivateKey);
                };
                let Some(qpl_tool_path) = self.qpl_tool_path.as_ref() else {
                    return Err(ProofCoordinatorError::Custom(
                        "Missing QPL tool path".to_string(),
                    ));
                };
                prepare_quote_prerequisites(
                    &self.eth_client,
                    &self.rpc_url,
                    &hex::encode(key.secret_bytes()),
                    &hex::encode(&payload),
                    qpl_tool_path,
                )
                .await
                .map_err(|e| {
                    ProofCoordinatorError::Custom(format!("Could not setup TDX key {e}"))
                })?;
                register_tdx_key(
                    &self.eth_client,
                    key,
                    self.on_chain_proposer_address,
                    payload,
                )
                .await?;
            }
            _ => {
                warn!("Setup requested for {prover_type}, which doesn't need setup.")
            }
        }

        let response = ProofData::prover_setup_ack();

        send_response(stream, &response).await?;
        info!("ProverSetupACK sent");
        Ok(())
    }
}

#[protocol]
pub trait ConnectionHandlerProtocol: Send + Sync {
    fn connection(&self, stream: Arc<TcpStream>, addr: SocketAddr) -> Result<(), ActorError>;
}

#[derive(Clone)]
struct ConnectionHandler {
    proof_coordinator: ProofCoordinator,
}

#[actor(protocol = ConnectionHandlerProtocol)]
impl ConnectionHandler {
    fn new(proof_coordinator: ProofCoordinator) -> Self {
        Self { proof_coordinator }
    }

    async fn spawn(
        proof_coordinator: ProofCoordinator,
        stream: Arc<TcpStream>,
        addr: SocketAddr,
    ) -> Result<(), ConnectionHandlerError> {
        let connection_handler = Self::new(proof_coordinator);
        let actor_ref = connection_handler.start();
        actor_ref
            .send(connection_handler_protocol::Connection { stream, addr })
            .map_err(ConnectionHandlerError::InternalError)
    }

    #[send_handler]
    async fn handle_connection_msg(
        &mut self,
        msg: connection_handler_protocol::Connection,
        ctx: &Context<Self>,
    ) {
        if let Err(err) = self.handle_connection(msg.stream).await {
            error!("Error handling connection from {}: {err}", msg.addr);
        } else {
            debug!("Connection from {} handled successfully", msg.addr);
        }
        ctx.stop();
    }

    async fn handle_connection(
        &mut self,
        stream: Arc<TcpStream>,
    ) -> Result<(), ProofCoordinatorError> {
        let mut buffer = Vec::new();
        // TODO: This should be fixed in https://github.com/lambdaclass/ethrex/issues/3316
        // (stream should not be wrapped in an Arc)
        if let Some(mut stream) = Arc::into_inner(stream) {
            stream.read_to_end(&mut buffer).await?;

            let data: Result<ProofData<ProverInputData>, _> = serde_json::from_slice(&buffer);
            match data {
                Ok(ProofData::InputRequest {
                    commit_hash,
                    prover_type,
                }) => {
                    if let Err(e) = self
                        .proof_coordinator
                        .handle_request(&mut stream, commit_hash, prover_type)
                        .await
                    {
                        error!("Failed to handle InputRequest: {e}");
                    }
                }
                Ok(ProofData::ProofSubmit {
                    id: batch_number,
                    proof: proof_bytes,
                }) => {
                    if let Err(e) = self
                        .proof_coordinator
                        .handle_submit(&mut stream, batch_number, proof_bytes)
                        .await
                    {
                        error!("Failed to handle ProofSubmit: {e}");
                    }
                }
                Ok(ProofData::ProverSetup {
                    prover_type,
                    payload,
                }) => {
                    if let Err(e) = self
                        .proof_coordinator
                        .handle_setup(&mut stream, prover_type, payload)
                        .await
                    {
                        error!("Failed to handle ProverSetup: {e}");
                    }
                }
                Ok(_) => {
                    warn!("Invalid request");
                }
                Err(e) => {
                    warn!("Failed to parse request: {e}");
                }
            }
            debug!("Connection closed");
        } else {
            error!("Unable to use stream");
        }
        Ok(())
    }
}

async fn send_response(
    stream: &mut TcpStream,
    response: &ProofData<ProverInputData>,
) -> Result<(), ProofCoordinatorError> {
    let buffer = serde_json::to_vec(response)?;
    stream
        .write_all(&buffer)
        .await
        .map_err(ProofCoordinatorError::ConnectionError)?;
    Ok(())
}
