use crate::backend::{BackendError, ProverBackend};
use ethrex_guest_program::input::ProgramInput;
use ethrex_l2_common::prover::{BatchProof, ProofData, ProofFormat, ProverInputData, ProverType};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::sleep,
};
use tracing::{debug, error, info, warn};
use url::Url;

pub struct ProverData {
    pub batch_number: u64,
    pub input: ProgramInput,
    pub format: ProofFormat,
}

/// The result of polling a proof coordinator for work.
pub enum InputRequest {
    /// A batch was assigned to this prover.
    Batch(Box<ProverData>),
    /// No work available right now (prover ahead of proposer, proof already
    /// exists, version mismatch). The prover should retry later.
    RetryLater,
    /// The coordinator permanently rejected this prover's type.
    /// The prover should skip this coordinator and continue with others.
    ProverTypeNotNeeded(ProverType),
}

/// Configuration for the prover pull loop.
pub struct ProverLoopConfig {
    pub proof_coordinator_endpoints: Vec<Url>,
    pub proving_time_ms: u64,
    pub timed: bool,
    pub commit_hash: String,
}

/// Trait for converting `ProverInputData` (from the coordinator) into `ProgramInput`
/// (for the backend). Different deployments (L2, L1/EIP-8025) may convert differently.
pub trait InputConverter: Send + Sync {
    fn convert(&self, input: ProverInputData) -> ProgramInput;
}

pub struct Prover<B: ProverBackend, C: InputConverter> {
    backend: B,
    converter: C,
    config: ProverLoopConfig,
}

impl<B: ProverBackend, C: InputConverter> Prover<B, C> {
    pub fn new(backend: B, converter: C, config: ProverLoopConfig) -> Self {
        Self {
            backend,
            converter,
            config,
        }
    }

    pub async fn start(&self) {
        info!(
            "Prover started for {:?}",
            self.config
                .proof_coordinator_endpoints
                .iter()
                .map(|url| url.to_string())
                .collect::<Vec<String>>()
        );
        loop {
            sleep(Duration::from_millis(self.config.proving_time_ms)).await;

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

                let batch_proof = self.prove_batch(
                    prover_data.input,
                    prover_data.format,
                    prover_data.batch_number,
                );
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
    }

    /// Prove a batch and return the batch proof.
    fn prove_batch(
        &self,
        input: ProgramInput,
        format: ProofFormat,
        batch_number: u64,
    ) -> Result<BatchProof, BackendError> {
        if self.config.timed {
            self.backend
                .prove_timed(input, format)
                .and_then(|(output, elapsed)| {
                    info!(
                        batch = batch_number,
                        proving_time_s = elapsed.as_secs(),
                        proving_time_ms =
                            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                        "Proved batch {} in {:.2?}",
                        batch_number,
                        elapsed
                    );
                    self.backend.to_batch_proof(output, format)
                })
        } else {
            self.backend
                .prove(input, format)
                .and_then(|output| {
                    info!(
                        batch = batch_number,
                        "Proved batch {}", batch_number
                    );
                    self.backend.to_batch_proof(output, format)
                })
        }
    }

    async fn request_new_input(&self, endpoint: &Url) -> Result<InputRequest, String> {
        let request =
            ProofData::batch_request(self.config.commit_hash.clone(), self.backend.prover_type());
        let response = connect_to_prover_server_wr(endpoint, &request)
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
        let program_input = self.converter.convert(input);
        Ok(InputRequest::Batch(Box::new(ProverData {
            batch_number,
            input: program_input,
            format,
        })))
    }

    async fn submit_proof(
        &self,
        endpoint: &Url,
        batch_number: u64,
        batch_proof: BatchProof,
    ) -> Result<(), String> {
        let submit = ProofData::proof_submit(batch_number, batch_proof);

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

pub async fn connect_to_prover_server_wr(
    endpoint: &Url,
    write: &ProofData,
) -> Result<ProofData, Box<dyn std::error::Error>> {
    debug!("Connecting with {endpoint}");
    let mut stream = TcpStream::connect(&*endpoint.socket_addrs(|| None)?).await?;
    debug!("Connection established!");

    stream.write_all(&serde_json::to_vec(&write)?).await?;
    stream.shutdown().await?;

    let mut buffer = Vec::new();
    stream.read_to_end(&mut buffer).await?;

    let response: Result<ProofData, _> = serde_json::from_slice(&buffer);
    Ok(response?)
}
