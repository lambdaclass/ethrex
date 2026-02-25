use crate::{
    backend::{BackendType, ExecBackend, ProverBackend},
    config::ProverConfig,
};
use ethrex_guest_program::input::ProgramInput;
use ethrex_l2::sequencer::utils::get_git_commit_hash;
use ethrex_l2_common::prover::{BatchProof, ProofData, ProofFormat, ProverType};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::sleep,
};
use tracing::{debug, error, info, warn};
use url::Url;

pub async fn start_prover(config: ProverConfig) {
    match config.backend {
        BackendType::Exec => {
            let prover = Prover::new(ExecBackend::new(), &config);
            prover.start().await;
        }
        #[cfg(feature = "sp1")]
        BackendType::SP1 => {
            use crate::backend::sp1::{PROVER_SETUP, Sp1Backend, init_prover_setup};
            // CudaProver builder internally calls block_on(), which panics inside a tokio
            // runtime. Spawn initialization on a separate OS thread to avoid this.
            #[cfg(feature = "gpu")]
            PROVER_SETUP.get_or_init(|| {
                let endpoint = config.sp1_server.clone();
                std::thread::spawn(move || init_prover_setup(endpoint))
                    .join()
                    .expect("Failed to initialize SP1 prover setup")
            });
            #[cfg(not(feature = "gpu"))]
            PROVER_SETUP.get_or_init(|| init_prover_setup(None));
            let prover = Prover::new(Sp1Backend::new(), &config);
            prover.start().await;
        }
        #[cfg(feature = "risc0")]
        BackendType::RISC0 => {
            use crate::backend::Risc0Backend;
            let prover = Prover::new(Risc0Backend::new(), &config);
            prover.start().await;
        }
        #[cfg(feature = "zisk")]
        BackendType::ZisK => {
            use crate::backend::ZiskBackend;
            let prover = Prover::new(ZiskBackend::new(), &config);
            prover.start().await;
        }
        #[cfg(feature = "openvm")]
        BackendType::OpenVM => {
            use crate::backend::OpenVmBackend;
            let prover = Prover::new(OpenVmBackend::new(), &config);
            prover.start().await;
        }
    }
}

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

struct Prover<B: ProverBackend> {
    backend: B,
    proof_coordinator_endpoints: Vec<Url>,
    proving_time_ms: u64,
    timed: bool,
    commit_hash: String,
}

impl<B: ProverBackend> Prover<B> {
    pub fn new(backend: B, cfg: &ProverConfig) -> Self {
        Self {
            backend,
            proof_coordinator_endpoints: cfg.proof_coordinators.clone(),
            proving_time_ms: cfg.proving_time_ms,
            timed: cfg.timed,
            commit_hash: get_git_commit_hash(),
        }
    }

    pub async fn start(&self) {
        info!(
            "Prover started for {:?}",
            self.proof_coordinator_endpoints
                .iter()
                .map(|url| url.to_string())
                .collect::<Vec<String>>()
        );
        loop {
            sleep(Duration::from_millis(self.proving_time_ms)).await;

            for endpoint in &self.proof_coordinator_endpoints {
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

                let batch_proof = if self.timed {
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
    }

    async fn request_new_input(&self, endpoint: &Url) -> Result<InputRequest, String> {
        let request =
            ProofData::batch_request(self.commit_hash.clone(), self.backend.prover_type());
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
        #[cfg(feature = "l2")]
        let input = ProgramInput {
            blocks: input.blocks,
            execution_witness: input.execution_witness,
            elasticity_multiplier: input.elasticity_multiplier,
            blob_commitment: input.blob_commitment,
            blob_proof: input.blob_proof,
            fee_configs: input.fee_configs,
        };
        #[cfg(not(feature = "l2"))]
        let input = ProgramInput {
            blocks: input.blocks,
            execution_witness: input.execution_witness,
        };
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

async fn connect_to_prover_server_wr(
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
