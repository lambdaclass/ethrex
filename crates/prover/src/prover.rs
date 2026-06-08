use crate::{
    backend::{BackendType, ExecBackend, ProverBackend},
    protocol::ProofData,
};
use ethrex_common::types::prover::{ProofFormat, ProverOutput, ProverType};
use ethrex_guest_program::input::ProgramInput;
use serde::{Serialize, de::DeserializeOwned};
use spawned_concurrency::{
    error::ActorError,
    protocol,
    tasks::{Actor, ActorStart as _, Backend, Context, Handler, send_after},
};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tracing::{debug, error, info, warn};
use url::Url;

struct ProverData {
    id: u64,
    input: ProgramInput,
    format: ProofFormat,
}

/// The result of polling a proof coordinator for work.
enum InputRequest {
    /// Work was assigned to this prover.
    Work(Box<ProverData>),
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

/// Protocol for the generic Prover actor.
#[protocol]
pub trait ProverProtocol: Send + Sync {
    /// Poll all coordinator endpoints for work, prove, and submit.
    /// After completing one cycle, reschedules itself via `send_after`.
    fn poll(&self) -> Result<(), ActorError>;
    /// Stop the prover gracefully.
    fn abort(&self) -> Result<(), ActorError>;
}

/// Generic prover that polls coordinator endpoints for work, proves, and submits.
///
/// Uses a periodic polling loop: each `Poll` message triggers one cycle across
/// all configured endpoints, then schedules the next `Poll` after the configured
/// delay. Send `Abort` to stop the prover cleanly.
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
                Ok(InputRequest::Work(data)) => *data,
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

            let prover_output = if self.config.timed {
                self.backend
                    .prove_timed(prover_data.input, prover_data.format)
                    .and_then(|(output, elapsed)| {
                        info!(
                            id = prover_data.id,
                            proving_time_s = elapsed.as_secs(),
                            proving_time_ms =
                                u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                            "Proved payload #{} in {:.2?}",
                            prover_data.id,
                            elapsed
                        );
                        self.backend.to_proof_bytes(output, prover_data.format)
                    })
            } else {
                self.backend
                    .prove(prover_data.input, prover_data.format)
                    .and_then(|output| {
                        info!(id = prover_data.id, "Proved payload #{}", prover_data.id);
                        self.backend.to_proof_bytes(output, prover_data.format)
                    })
            };
            let Ok(prover_output) = prover_output.inspect_err(|e| error!("{e}")) else {
                continue;
            };

            let _ = self
                .submit_proof(endpoint, prover_data.id, prover_output)
                .await
                .inspect_err(|e|
                // TODO: Retry?
                warn!(%endpoint, "Failed to submit proof: {e}"));
        }
    }

    async fn request_new_input(&self, endpoint: &Url) -> Result<InputRequest, String> {
        let request: ProofData<I> =
            ProofData::input_request(self.config.commit_hash.clone(), self.backend.prover_type());
        let response: ProofData<I> = connect_to_prover_server_wr(endpoint, &request)
            .await
            .map_err(|e| format!("Failed to get Response: {e}"))?;

        let (id, input, format) = match response {
            ProofData::InputResponse { id, input, format } => (id, input, format),
            ProofData::VersionMismatch => {
                warn!(
                    "Version mismatch: the next payload to prove was built with a different code \
                     version. This prover may need to be updated."
                );
                return Ok(InputRequest::RetryLater);
            }
            ProofData::ProverTypeNotNeeded { prover_type } => {
                return Ok(InputRequest::ProverTypeNotNeeded(prover_type));
            }
            _ => return Err("Expecting ProofData::Response".to_owned()),
        };

        let (Some(id), Some(input), Some(format)) = (id, input, format) else {
            debug!(
                %endpoint,
                "No pending work, the prover may be ahead of the proposer"
            );
            return Ok(InputRequest::RetryLater);
        };

        info!(%endpoint, "Received payload #{id}");
        let input: ProgramInput = input.into();
        Ok(InputRequest::Work(Box::new(ProverData {
            id,
            input,
            format,
        })))
    }

    async fn submit_proof(
        &self,
        endpoint: &Url,
        id: u64,
        prover_output: ProverOutput,
    ) -> Result<(), String> {
        let submit: ProofData<I> = ProofData::proof_submit(id, prover_output);

        let ProofData::ProofSubmitACK { id } = connect_to_prover_server_wr(endpoint, &submit)
            .await
            .map_err(|e| format!("Failed to get SubmitAck: {e}"))?
        else {
            return Err("Expecting ProofData::SubmitAck".to_owned());
        };

        info!(%endpoint, "Proof for payload #{id} accepted");
        Ok(())
    }
}

// Manual Actor + Handler impls because `#[actor]` doesn't support generic impl blocks.
impl<B, I> Actor for Prover<B, I>
where
    B: ProverBackend + Send + Sync + 'static,
    I: Into<ProgramInput> + Serialize + DeserializeOwned + Send + Sync + 'static,
{
}

impl<B, I> Handler<prover_protocol::Poll> for Prover<B, I>
where
    B: ProverBackend + Send + Sync + 'static,
    I: Into<ProgramInput> + Serialize + DeserializeOwned + Send + Sync + 'static,
{
    async fn handle(&mut self, _msg: prover_protocol::Poll, ctx: &Context<Self>) {
        self.poll_endpoints().await;
        send_after(
            Duration::from_millis(self.config.proving_time_ms),
            ctx.clone(),
            prover_protocol::Poll,
        );
    }
}

impl<B, I> Handler<prover_protocol::Abort> for Prover<B, I>
where
    B: ProverBackend + Send + Sync + 'static,
    I: Into<ProgramInput> + Serialize + DeserializeOwned + Send + Sync + 'static,
{
    async fn handle(&mut self, _msg: prover_protocol::Abort, ctx: &Context<Self>) {
        // Stopping the actor ends the blocking runner cleanly.
        ctx.stop();
    }
}

/// Starts the prover with the appropriate backend based on the given config.
///
/// The caller provides the `ProverPullConfig` and the type parameter `I` determines
/// the input type used in the protocol.
///
/// The prover runs as an actor on a blocking thread (via `Backend::Blocking`) since
/// proving is CPU-intensive. This function blocks until the prover is stopped.
pub async fn start_prover<I>(backend_type: BackendType, config: ProverPullConfig)
where
    I: Into<ProgramInput> + Serialize + DeserializeOwned + Send + Sync + 'static,
{
    match backend_type {
        BackendType::Exec => {
            let prover: Prover<ExecBackend, I> = Prover::new(ExecBackend::new(), config);
            let actor_ref = prover.start_with_backend(Backend::Blocking);
            let _ = actor_ref.send(prover_protocol::Poll);
            actor_ref.join().await;
        }
        #[cfg(feature = "sp1")]
        BackendType::SP1 => {
            use crate::backend::sp1::{PROVER_SETUP, Sp1Backend, init_prover_setup};
            PROVER_SETUP.get_or_init(|| init_prover_setup(None));
            let prover: Prover<Sp1Backend, I> = Prover::new(Sp1Backend::new(), config);
            let actor_ref = prover.start_with_backend(Backend::Blocking);
            let _ = actor_ref.send(prover_protocol::Poll);
            actor_ref.join().await;
        }
        #[cfg(feature = "risc0")]
        BackendType::RISC0 => {
            use crate::backend::Risc0Backend;
            let prover: Prover<Risc0Backend, I> = Prover::new(Risc0Backend::new(), config);
            let actor_ref = prover.start_with_backend(Backend::Blocking);
            let _ = actor_ref.send(prover_protocol::Poll);
            actor_ref.join().await;
        }
        #[cfg(feature = "zisk")]
        BackendType::ZisK => {
            use crate::backend::ZiskBackend;
            let prover: Prover<ZiskBackend, I> = Prover::new(ZiskBackend::new(), config);
            let actor_ref = prover.start_with_backend(Backend::Blocking);
            let _ = actor_ref.send(prover_protocol::Poll);
            actor_ref.join().await;
        }
        #[cfg(feature = "openvm")]
        BackendType::OpenVM => {
            use crate::backend::OpenVmBackend;
            let prover: Prover<OpenVmBackend, I> = Prover::new(OpenVmBackend::new(), config);
            let actor_ref = prover.start_with_backend(Backend::Blocking);
            let _ = actor_ref.send(prover_protocol::Poll);
            actor_ref.join().await;
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
