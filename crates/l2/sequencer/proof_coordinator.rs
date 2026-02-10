use crate::SequencerConfig;
use crate::sequencer::errors::{ConnectionHandlerError, ProofCoordinatorError};
use crate::sequencer::setup::{prepare_quote_prerequisites, register_tdx_key};
use crate::sequencer::utils::get_git_commit_hash;
use bytes::Bytes;
use ethrex_common::Address;
use ethrex_l2_common::prover::{BatchProof, ProofData, ProofFormat, ProverInputData, ProverType};
use ethrex_metrics::metrics;
use ethrex_rpc::clients::eth::EthClient;
use ethrex_storage_rollup::StoreRollup;
use secp256k1::SecretKey;
use spawned_concurrency::messages::Unused;
use spawned_concurrency::tasks::{CastResponse, GenServer, GenServerHandle};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tracing::{debug, error, info, warn};

#[cfg(feature = "metrics")]
use ethrex_metrics::l2::metrics::METRICS;

#[derive(Clone)]
pub enum ProofCordInMessage {
    Listen { listener: Arc<TcpListener> },
}

#[derive(Clone, PartialEq)]
pub enum ProofCordOutMessage {
    Done,
}

#[derive(Clone)]
pub struct ProofCoordinator {
    listen_ip: IpAddr,
    port: u16,
    eth_client: EthClient,
    on_chain_proposer_address: Address,
    rollup_store: StoreRollup,
    rpc_url: String,
    tdx_private_key: Option<SecretKey>,
    // Used again in PR #6157; kept here to simplify merge conflicts.
    #[allow(dead_code)]
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
            listen_ip: config.proof_coordinator.listen_ip,
            port: config.proof_coordinator.listen_port,
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
        let state = Self::new(&cfg, rollup_store, needed_proof_types)?;
        let listener =
            Arc::new(TcpListener::bind(format!("{}:{}", state.listen_ip, state.port)).await?);
        let mut proof_coordinator = ProofCoordinator::start(state);
        let _ = proof_coordinator
            .cast(ProofCordInMessage::Listen { listener })
            .await;
        Ok(())
    }

    async fn handle_listens(&self, listener: Arc<TcpListener>) {
        info!("Starting TCP server at {}:{}.", self.listen_ip, self.port);
        loop {
            let res = listener.accept().await;
            match res {
                Ok((stream, addr)) => {
                    // Cloning the ProofCoordinatorState structure to use the handle_connection() fn
                    // in every spawned task.
                    // The important fields are `Store` and `EthClient`
                    // Both fields are wrapped with an Arc, making it possible to clone
                    // the entire structure.
                    let _ = ConnectionHandler::spawn(self.clone(), stream, addr)
                        .await
                        .inspect_err(|err| {
                            error!("Error starting ConnectionHandler: {err}");
                        });
                }
                Err(e) => {
                    error!("Failed to accept connection: {e}");
                }
            }

            debug!("Connection closed");
        }
    }

    async fn next_batch_to_assign(
        &self,
        commit_hash: &str,
        prover_type: ProverType,
    ) -> Result<Option<(u64, ProverInputData)>, ProofCoordinatorError> {
        let base_batch = 1 + self.rollup_store.get_latest_sent_batch_proof().await?;

        loop {
            // Lock briefly to find and claim a candidate
            let candidate = {
                let mut assignments = self.assignments.lock().map_err(|_| {
                    ProofCoordinatorError::Custom("Assignment lock poisoned".to_string())
                })?;

                assignments.retain(|&(batch, _), _| batch >= base_batch);

                let now = Instant::now();
                let mut batch = base_batch;

                let key = |b| (b, prover_type);
                loop {
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
        info!("BatchRequest received from {prover_type} prover");

        if commit_hash != self.git_commit_hash {
            debug!(
                "Mismatch on prover version. Expected: {}, got: {}. Looking for batches left to prove",
                self.git_commit_hash, commit_hash
            );
        }

        let Some((batch_to_prove, input)) =
            self.next_batch_to_assign(&commit_hash, prover_type).await?
        else {
            debug!("No batch available for this version, sending empty BatchResponse");
            let response = ProofData::empty_batch_response();
            send_response(stream, &response).await?;
            info!("Empty BatchResponse sent (no batch for version)");
            return Ok(());
        };

        debug!("Sending BatchResponse for block_number: {batch_to_prove}");
        let format = if self.aligned {
            ProofFormat::Compressed
        } else {
            ProofFormat::Groth16
        };
        let response = ProofData::batch_response(batch_to_prove, input, format);

        send_response(stream, &response).await?;
        info!("BatchResponse sent for batch number: {batch_to_prove}");

        Ok(())
    }

    async fn handle_submit(
        &self,
        stream: &mut TcpStream,
        batch_number: u64,
        batch_proof: BatchProof,
    ) -> Result<(), ProofCoordinatorError> {
        info!("ProofSubmit received for batch number: {batch_number}");

        // Check if we have a proof for this batch and prover type
        let prover_type = batch_proof.prover_type();
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
            // If not, store it
            self.rollup_store
                .store_proof_by_batch_and_type(batch_number, prover_type, batch_proof)
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

impl GenServer for ProofCoordinator {
    type CallMsg = Unused;
    type CastMsg = ProofCordInMessage;
    type OutMsg = ProofCordOutMessage;
    type Error = ProofCoordinatorError;

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            ProofCordInMessage::Listen { listener } => {
                self.handle_listens(listener).await;
            }
        }
        CastResponse::Stop
    }
}

#[derive(Clone)]
struct ConnectionHandler {
    proof_coordinator: ProofCoordinator,
}

impl ConnectionHandler {
    fn new(proof_coordinator: ProofCoordinator) -> Self {
        Self { proof_coordinator }
    }

    async fn spawn(
        proof_coordinator: ProofCoordinator,
        stream: TcpStream,
        addr: SocketAddr,
    ) -> Result<(), ConnectionHandlerError> {
        let mut connection_handler = Self::new(proof_coordinator).start();
        connection_handler
            .cast(ConnInMessage::Connection {
                stream: Arc::new(stream),
                addr,
            })
            .await
            .map_err(ConnectionHandlerError::InternalError)
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

            let data: Result<ProofData, _> = serde_json::from_slice(&buffer);
            match data {
                Ok(ProofData::BatchRequest {
                    commit_hash,
                    prover_type,
                }) => {
                    if let Err(e) = self
                        .proof_coordinator
                        .handle_request(&mut stream, commit_hash, prover_type)
                        .await
                    {
                        error!("Failed to handle BatchRequest: {e}");
                    }
                }
                Ok(ProofData::ProofSubmit {
                    batch_number,
                    batch_proof,
                }) => {
                    if let Err(e) = self
                        .proof_coordinator
                        .handle_submit(&mut stream, batch_number, batch_proof)
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

#[derive(Clone)]
pub enum ConnInMessage {
    Connection {
        stream: Arc<TcpStream>,
        addr: SocketAddr,
    },
}

#[derive(Clone, PartialEq)]
pub enum ConnOutMessage {
    Done,
}

impl GenServer for ConnectionHandler {
    type CallMsg = Unused;
    type CastMsg = ConnInMessage;
    type OutMsg = ConnOutMessage;
    type Error = ProofCoordinatorError;

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            ConnInMessage::Connection { stream, addr } => {
                if let Err(err) = self.handle_connection(stream).await {
                    error!("Error handling connection from {addr}: {err}");
                } else {
                    debug!("Connection from {addr} handled successfully");
                }
            }
        }
        CastResponse::Stop
    }
}

async fn send_response(
    stream: &mut TcpStream,
    response: &ProofData,
) -> Result<(), ProofCoordinatorError> {
    let buffer = serde_json::to_vec(response)?;
    stream
        .write_all(&buffer)
        .await
        .map_err(ProofCoordinatorError::ConnectionError)?;
    Ok(())
}
