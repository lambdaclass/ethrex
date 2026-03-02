use std::sync::Arc;
use std::time::Duration;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::sleep,
};
use tracing::{debug, error, info, warn};
use url::Url;

use ethrex_guest_program::input::ProgramInput;
use ethrex_guest_program::programs::{EvmL2GuestProgram, TokammonGuestProgram, ZkDexGuestProgram};
use ethrex_l2::sequencer::utils::get_git_commit_hash;
use ethrex_l2_common::prover::{BatchProof, ProofData, ProofFormat, ProverType};

use crate::backend::{BackendError, BackendType, ExecBackend, ProverBackend};
use crate::config::ProverConfig;
use crate::programs_config::ProgramsConfig;
use crate::registry::GuestProgramRegistry;

/// Create a guest program registry based on runtime config.
///
/// If `config_path` is `None`, all built-in programs are registered.
/// Otherwise, only the programs listed in the config file are registered.
fn create_registry(config_path: Option<&str>) -> GuestProgramRegistry {
    let config = config_path
        .map(|p| {
            ProgramsConfig::load(p).unwrap_or_else(|e| {
                warn!("Failed to load programs config from {p}: {e}, using defaults");
                ProgramsConfig::default()
            })
        })
        .unwrap_or_default();

    let mut registry = GuestProgramRegistry::new(&config.default_program);

    let all_programs: Vec<(String, Arc<dyn ethrex_guest_program::traits::GuestProgram>)> = vec![
        ("evm-l2".to_string(), Arc::new(EvmL2GuestProgram)),
        ("zk-dex".to_string(), Arc::new(ZkDexGuestProgram)),
        ("tokamon".to_string(), Arc::new(TokammonGuestProgram)),
    ];

    for (id, program) in all_programs {
        if config.enabled_programs.contains(&id) {
            registry.register(program);
        }
    }

    registry
}

pub async fn start_prover(config: ProverConfig) {
    let registry = create_registry(config.programs_config_path.as_deref());
    match config.backend {
        BackendType::Exec => {
            let prover = Prover::new(ExecBackend::new(), &config, registry);
            prover.start().await;
        }
        #[cfg(feature = "sp1")]
        BackendType::SP1 => {
            use crate::backend::sp1::{PROVER_SETUP, Sp1Backend, init_prover_setup};
            #[cfg(feature = "gpu")]
            PROVER_SETUP.get_or_init(|| init_prover_setup(config.sp1_server.clone()));
            #[cfg(not(feature = "gpu"))]
            PROVER_SETUP.get_or_init(|| init_prover_setup(None));
            let prover = Prover::new(Sp1Backend::new(), &config, registry);
            prover.start().await;
        }
        #[cfg(feature = "risc0")]
        BackendType::RISC0 => {
            use crate::backend::Risc0Backend;
            let prover = Prover::new(Risc0Backend::new(), &config, registry);
            prover.start().await;
        }
        #[cfg(feature = "zisk")]
        BackendType::ZisK => {
            use crate::backend::ZiskBackend;
            let prover = Prover::new(ZiskBackend::new(), &config, registry);
            prover.start().await;
        }
        #[cfg(feature = "openvm")]
        BackendType::OpenVM => {
            use crate::backend::OpenVmBackend;
            let prover = Prover::new(OpenVmBackend::new(), &config, registry);
            prover.start().await;
        }
    }
}

struct ProverData {
    batch_number: u64,
    input: ProgramInput,
    format: ProofFormat,
    program_id: String,
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
    registry: GuestProgramRegistry,
    proof_coordinator_endpoints: Vec<Url>,
    proving_time_ms: u64,
    timed: bool,
    commit_hash: String,
}

impl<B: ProverBackend> Prover<B> {
    pub fn new(backend: B, cfg: &ProverConfig, registry: GuestProgramRegistry) -> Self {
        Self {
            backend,
            registry,
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

                let batch_proof = self.prove_batch(
                    prover_data.input,
                    prover_data.format,
                    prover_data.batch_number,
                    &prover_data.program_id,
                );
                let Ok(batch_proof) = batch_proof.inspect_err(|e| error!("{e}")) else {
                    continue;
                };

                let _ = self
                    .submit_proof(
                        endpoint,
                        prover_data.batch_number,
                        batch_proof,
                        &prover_data.program_id,
                    )
                    .await
                    .inspect_err(|e|
                    // TODO: Retry?
                    warn!(%endpoint, "Failed to submit proof: {e}"));
            }
        }
    }

    /// Prove a batch, trying the registry-based ELF path first and falling
    /// back to the legacy `prove()` path when no ELF is available (e.g. exec
    /// backend, or ELF not compiled for this backend).
    fn prove_batch(
        &self,
        input: ProgramInput,
        format: ProofFormat,
        batch_number: u64,
        program_id: &str,
    ) -> Result<BatchProof, BackendError> {
        // Try to resolve an ELF binary from the registry for this program + backend.
        let elf_and_program = self.registry.get(program_id).and_then(|program| {
            program
                .elf(self.backend.backend_name())
                .map(|elf| (program, elf))
        });

        if let Some((program, elf)) = elf_and_program {
            // Registry-based path: serialize input to raw bytes, then prove_with_elf.
            let input_bytes = self.backend.serialize_raw(&input)?;
            let serialized = program
                .serialize_input(input_bytes.as_slice())
                .map_err(|e| BackendError::serialization(e.to_string()))?;

            // Enforce input size limit.
            let limits = program.resource_limits();
            if let Some(max) = limits.max_input_bytes
                && serialized.len() > max
            {
                return Err(BackendError::resource_limit(format!(
                    "input size {} bytes exceeds limit of {} bytes for program '{}'",
                    serialized.len(),
                    max,
                    program_id
                )));
            }

            if self.timed {
                let (output, elapsed) =
                    self.backend
                        .prove_with_elf_timed(elf, &serialized, format)?;
                // Enforce proving duration limit.
                if let Some(max_dur) = limits.max_proving_duration
                    && elapsed > max_dur
                {
                    return Err(BackendError::resource_limit(format!(
                        "proving took {elapsed:.2?} which exceeds limit of {max_dur:.2?} for program '{program_id}'"
                    )));
                }
                info!(
                    batch = batch_number,
                    proving_time_s = elapsed.as_secs(),
                    proving_time_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                    "Proved batch {batch_number} in {elapsed:.2?} (program: {program_id}, elf)"
                );
                self.backend.to_batch_proof(output, format)
            } else {
                let start = std::time::Instant::now();
                let output = self.backend.prove_with_elf(elf, &serialized, format)?;
                // Enforce proving duration limit even in untimed mode.
                if let Some(max_dur) = limits.max_proving_duration {
                    let elapsed = start.elapsed();
                    if elapsed > max_dur {
                        return Err(BackendError::resource_limit(format!(
                            "proving took {elapsed:.2?} which exceeds limit of {max_dur:.2?} for program '{program_id}'"
                        )));
                    }
                }
                info!(
                    batch = batch_number,
                    "Proved batch {batch_number} (program: {program_id}, elf)"
                );
                self.backend.to_batch_proof(output, format)
            }
        } else {
            // Legacy path: no ELF available, use prove() with ProgramInput directly.
            if self.timed {
                let (output, elapsed) = self.backend.prove_timed(input, format)?;
                info!(
                    batch = batch_number,
                    proving_time_s = elapsed.as_secs(),
                    proving_time_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                    "Proved batch {batch_number} in {elapsed:.2?} (program: {program_id}, legacy)"
                );
                self.backend.to_batch_proof(output, format)
            } else {
                let output = self.backend.prove(input, format)?;
                info!(
                    batch = batch_number,
                    "Proved batch {batch_number} (program: {program_id}, legacy)"
                );
                self.backend.to_batch_proof(output, format)
            }
        }
    }

    async fn request_new_input(&self, endpoint: &Url) -> Result<InputRequest, String> {
        let supported = self
            .registry
            .program_ids()
            .iter()
            .map(|s| s.to_string())
            .collect();
        let request = ProofData::batch_request_with_programs(
            self.commit_hash.clone(),
            self.backend.prover_type(),
            supported,
        );
        let response = connect_to_prover_server_wr(endpoint, &request)
            .await
            .map_err(|e| format!("Failed to get Response: {e}"))?;

        let (batch_number, input, format, program_id) = match response {
            ProofData::BatchResponse {
                batch_number,
                input,
                format,
                program_id,
            } => (batch_number, input, format, program_id),
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

        // Default to "evm-l2" when the coordinator doesn't specify a program.
        let program_id = program_id.unwrap_or_else(|| "evm-l2".to_string());

        info!(%endpoint, "Received Response for batch_number: {batch_number} (program: {program_id})");
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
            program_id,
        })))
    }

    async fn submit_proof(
        &self,
        endpoint: &Url,
        batch_number: u64,
        batch_proof: BatchProof,
        program_id: &str,
    ) -> Result<(), String> {
        let submit =
            ProofData::proof_submit_with_program(batch_number, batch_proof, program_id.to_string());

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
