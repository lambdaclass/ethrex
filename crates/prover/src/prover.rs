use crate::{
    backend::{BackendType, ExecBackend, ProverBackend},
    protocol::ProofData,
};
use ethrex_guest_program::input::ProgramInput;
use ethrex_l2_common::prover::{BatchProof, ProofFormat, ProverType};
use serde::{Serialize, de::DeserializeOwned};
use spawned_concurrency::messages::Unused;
use spawned_concurrency::tasks::{CastResponse, GenServer, GenServerHandle, send_after};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tracing::{debug, error, info, warn};
use url::Url;

struct ProverData {
    batch_number: u64,
    input: ProgramInput,
    format: ProofFormat,
}

/// The result of polling a proof coordinator for work.
enum InputRequest {
    /// A batch was assigned to this prover.
    Batch(Box<ProverData>),
    /// No work available right now (prover ahead of proposer, proof already
    /// exists, version mismatch). The prover should retry later.
    RetryLater,
    /// The coordinator permanently rejected this prover's type.
    /// The prover should skip this coordinator and continue with others.
    ProverTypeNotNeeded(ProverType),
}

/// Configuration for the generic prover pull loop.
pub struct ProverPullConfig {
    pub proof_coordinator_endpoints: Vec<Url>,
    pub proving_time_ms: u64,
    pub timed: bool,
    pub commit_hash: String,
}

/// Messages the prover GenServer accepts via `cast`.
#[derive(Clone)]
pub enum InMessage {
    /// Poll all coordinator endpoints for work, prove, and submit.
    /// After completing one cycle, reschedules itself via `send_after`.
    Poll,
    /// Stop the prover gracefully.
    Abort,
}

/// Generic prover that polls coordinator endpoints for work, proves, and submits.
///
/// Implements `GenServer` with a periodic polling loop: each `Poll` message triggers
/// one cycle across all configured endpoints, then schedules the next `Poll` after
/// the configured delay. Send `Abort` to stop the prover cleanly.
///
/// - `B` is the backend (SP1, RISC0, Exec, etc.)
/// - `I` is the input type received from the coordinator (e.g., `ProverInputData` for L2,
///   `ProgramInput` for L1). Must implement `Into<ProgramInput>`.
pub struct Prover<B: ProverBackend, I> {
    backend: B,
    config: ProverPullConfig,
    _input: std::marker::PhantomData<I>,
}

impl<B: ProverBackend, I> Prover<B, I>
where
    I: Into<ProgramInput> + Serialize + DeserializeOwned + Send + Sync + 'static,
{
    pub fn new(backend: B, config: ProverPullConfig) -> Self {
        Self {
            backend,
            config,
            _input: std::marker::PhantomData,
        }
    }

    /// Run one polling cycle: iterate all coordinator endpoints, request work,
    /// prove, and submit results.
    async fn poll_endpoints(&self) {
        for endpoint in &self.config.proof_coordinator_endpoints {
            let prover_data = match self.request_new_input(endpoint).await {
                Ok(InputRequest::Batch(data)) => *data,
                Ok(InputRequest::RetryLater) => continue,
                Ok(InputRequest::ProverTypeNotNeeded(prover_type)) => {
                    error!(
                        %endpoint,
                        "Proof coordinator does not need {prover_type} proofs. \
                         This prover's backend is not in the required proof types \
                         for this deployment."
                    );
                    continue;
                }
                Err(e) => {
                    error!(%endpoint, "Failed to request new data: {e}");
                    continue;
                }
            };

            let batch_proof = if self.config.timed {
                self.backend
                    .prove_timed(prover_data.input, prover_data.format)
                    .and_then(|(output, elapsed)| {
                        info!(
                            batch = prover_data.batch_number,
                            proving_time_s = elapsed.as_secs(),
                            proving_time_ms =
                                u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                            "Proved batch {} in {:.2?}",
                            prover_data.batch_number,
                            elapsed
                        );
                        self.backend.to_batch_proof(output, prover_data.format)
                    })
            } else {
                self.backend
                    .prove(prover_data.input, prover_data.format)
                    .and_then(|output| {
                        info!(
                            batch = prover_data.batch_number,
                            "Proved batch {}", prover_data.batch_number
                        );
                        self.backend.to_batch_proof(output, prover_data.format)
                    })
            };
            let Ok(batch_proof) = batch_proof.inspect_err(|e| error!("{e}")) else {
                continue;
            };

            let _ = self
                .submit_proof(endpoint, prover_data.batch_number, batch_proof)
                .await
                .inspect_err(|e|
                // TODO: Retry?
                warn!(%endpoint, "Failed to submit proof: {e}"));
        }
    }

    async fn request_new_input(&self, endpoint: &Url) -> Result<InputRequest, String> {
        let request: ProofData<I> = ProofData::batch_request(
            self.config.commit_hash.clone(),
            self.backend.prover_type(),
        );
        let response: ProofData<I> = connect_to_prover_server_wr(endpoint, &request)
            .await
            .map_err(|e| format!("Failed to get Response: {e}"))?;

        let (batch_number, input, format) = match response {
            ProofData::BatchResponse {
                batch_number,
                input,
                format,
            } => (batch_number, input, format),
            ProofData::VersionMismatch => {
                warn!(
                    "Version mismatch: the next batch to prove was built with a different code \
                     version. This prover may need to be updated."
                );
                return Ok(InputRequest::RetryLater);
            }
            ProofData::ProverTypeNotNeeded { prover_type } => {
                return Ok(InputRequest::ProverTypeNotNeeded(prover_type));
            }
            _ => return Err("Expecting ProofData::Response".to_owned()),
        };

        let (Some(batch_number), Some(input), Some(format)) = (batch_number, input, format) else {
            debug!(
                %endpoint,
                "No batches to prove right now, the prover may be ahead of the proposer"
            );
            return Ok(InputRequest::RetryLater);
        };

        info!(%endpoint, "Received Response for batch_number: {batch_number}");
        let input: ProgramInput = input.into();
        Ok(InputRequest::Batch(Box::new(ProverData {
            batch_number,
            input,
            format,
        })))
    }

    async fn submit_proof(
        &self,
        endpoint: &Url,
        batch_number: u64,
        batch_proof: BatchProof,
    ) -> Result<(), String> {
        let submit: ProofData<I> = ProofData::proof_submit(batch_number, batch_proof);

        let ProofData::ProofSubmitACK { batch_number } =
            connect_to_prover_server_wr(endpoint, &submit)
                .await
                .map_err(|e| format!("Failed to get SubmitAck: {e}"))?
        else {
            return Err("Expecting ProofData::SubmitAck".to_owned());
        };

        info!(%endpoint, "Received submit ack for batch_number: {batch_number}");
        Ok(())
    }
}

impl<B, I> GenServer for Prover<B, I>
where
    B: ProverBackend + Send + Sync + 'static,
    I: Into<ProgramInput> + Serialize + DeserializeOwned + Send + Sync + 'static,
{
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = Unused;
    type Error = crate::BackendError;

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            InMessage::Poll => {
                self.poll_endpoints().await;
                send_after(
                    Duration::from_millis(self.config.proving_time_ms),
                    handle.clone(),
                    InMessage::Poll,
                );
                CastResponse::NoReply
            }
            InMessage::Abort => {
                // start_blocking keeps the prover loop alive even if the caller aborts the task.
                // Returning CastResponse::Stop ends the blocking runner cleanly.
                CastResponse::Stop
            }
        }
    }
}

/// Starts the prover with the appropriate backend based on the given config.
///
/// The caller provides the `ProverPullConfig` and the type parameter `I` determines
/// the input type used in the protocol.
///
/// The prover runs as a GenServer on a blocking thread (via `start_blocking`) since
/// proving is CPU-intensive. This function blocks until the prover is stopped.
pub async fn start_prover<I>(backend_type: BackendType, config: ProverPullConfig)
where
    I: Into<ProgramInput> + Serialize + DeserializeOwned + Send + Sync + 'static,
{
    match backend_type {
        BackendType::Exec => {
            let prover: Prover<ExecBackend, I> = Prover::new(ExecBackend::new(), config);
            let mut handle = prover.start_blocking();
            let _ = handle.cast(InMessage::Poll).await;
            handle.cancellation_token().cancelled().await;
        }
        #[cfg(feature = "sp1")]
        BackendType::SP1 => {
            use crate::backend::sp1::{PROVER_SETUP, Sp1Backend, init_prover_setup};
            PROVER_SETUP.get_or_init(|| init_prover_setup(None));
            let prover: Prover<Sp1Backend, I> = Prover::new(Sp1Backend::new(), config);
            let mut handle = prover.start_blocking();
            let _ = handle.cast(InMessage::Poll).await;
            handle.cancellation_token().cancelled().await;
        }
        #[cfg(feature = "risc0")]
        BackendType::RISC0 => {
            use crate::backend::Risc0Backend;
            let prover: Prover<Risc0Backend, I> = Prover::new(Risc0Backend::new(), config);
            let mut handle = prover.start_blocking();
            let _ = handle.cast(InMessage::Poll).await;
            handle.cancellation_token().cancelled().await;
        }
        #[cfg(feature = "zisk")]
        BackendType::ZisK => {
            use crate::backend::ZiskBackend;
            let prover: Prover<ZiskBackend, I> = Prover::new(ZiskBackend::new(), config);
            let mut handle = prover.start_blocking();
            let _ = handle.cast(InMessage::Poll).await;
            handle.cancellation_token().cancelled().await;
        }
        #[cfg(feature = "openvm")]
        BackendType::OpenVM => {
            use crate::backend::OpenVmBackend;
            let prover: Prover<OpenVmBackend, I> = Prover::new(OpenVmBackend::new(), config);
            let mut handle = prover.start_blocking();
            let _ = handle.cast(InMessage::Poll).await;
            handle.cancellation_token().cancelled().await;
        }
    }
}

async fn connect_to_prover_server_wr<I: Serialize + DeserializeOwned>(
    endpoint: &Url,
    write: &ProofData<I>,
) -> Result<ProofData<I>, Box<dyn std::error::Error>> {
    debug!("Connecting with {endpoint}");
    let mut stream = TcpStream::connect(&*endpoint.socket_addrs(|| None)?).await?;
    debug!("Connection established!");

    stream.write_all(&serde_json::to_vec(&write)?).await?;
    stream.shutdown().await?;

    let mut buffer = Vec::new();
    stream.read_to_end(&mut buffer).await?;

    let response: Result<ProofData<I>, _> = serde_json::from_slice(&buffer);
    Ok(response?)
}
