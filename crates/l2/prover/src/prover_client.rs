use ethrex_l2::sequencer::proof_coordinator::{ProofData, get_commit_hash};
use ethrex_l2_common::prover::BatchProof;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::sleep,
};
use tracing::{debug, error, info, warn};

#[cfg(not(any(feature = "risc0", feature = "sp1")))]
use ethrex_prover_lib::backend::exec::ExecBackend;
#[cfg(feature = "risc0")]
use ethrex_prover_lib::backend::risc0::RISC0Backend;
#[cfg(feature = "sp1")]
use ethrex_prover_lib::backend::sp1::SP1Backend;

use ethrex_prover_lib::{backend::ProverBackend, guest_program::input::ProgramInput};

pub async fn start_prover(config: ProverConfig) {
    #[cfg(not(any(feature = "risc0", feature = "sp1")))]
    let backend = ExecBackend {};

    #[cfg(feature = "risc0")]
    let backend = RISC0Backend {};

    #[cfg(feature = "sp1")]
    let backend = SP1Backend {};

    let prover_worker = Prover::new(config, Arc::new(backend));

    prover_worker.start().await;
}

pub struct ProverConfig {
    pub http_addr: String,
    pub http_port: u16,
    pub proving_time_ms: u64,
    pub aligned_mode: bool,
}

struct ProverData {
    batch_number: u64,
    input: ProgramInput,
}

// PI = Program Input
// PO = Program Output
// E = Error
struct Prover<PO, E> {
    backend: Arc<dyn ProverBackend<ProgramOutput = PO, Error = E>>,
    prover_server_endpoint: String,
    proving_time_ms: u64,
    aligned_mode: bool,
    commit_hash: String,
}

// PO = Program Output
// E = Error
impl<PO, E> Prover<PO, E>
where
    E: std::fmt::Display,
{
    pub fn new(
        cfg: ProverConfig,
        backend: Arc<dyn ProverBackend<ProgramOutput = PO, Error = E>>,
    ) -> Self {
        Self {
            backend,
            prover_server_endpoint: format!("{}:{}", cfg.http_addr, cfg.http_port),
            proving_time_ms: cfg.proving_time_ms,
            aligned_mode: cfg.aligned_mode,
            commit_hash: get_commit_hash(),
        }
    }

    pub async fn start(&self) {
        info!("Prover started on {}", self.prover_server_endpoint);
        // Build the prover depending on the prover_type passed as argument.
        loop {
            sleep(Duration::from_millis(self.proving_time_ms)).await;

            let Ok(prover_data) = self
                .request_new_input()
                .await
                .inspect_err(|e| error!("Failed to request new data: {e}"))
            else {
                continue;
            };

            let Some(prover_data) = prover_data else {
                continue;
            };

            // If we get the input
            // Generate the Proof
            let Ok(batch_proof) = self
                .backend
                .prove(prover_data.input, self.aligned_mode)
                .and_then(|output| self.backend.to_batch_proof(output, self.aligned_mode))
                .inspect_err(|e| error!("{e}"))
            else {
                continue;
            };

            let _ = self
                .submit_proof(prover_data.batch_number, batch_proof)
                .await
                .inspect_err(|e|
                    // TODO: Retry?
                    warn!("Failed to submit proof: {e}"));
        }
    }

    async fn request_new_input(&self) -> Result<Option<ProverData>, String> {
        // Request the input with the correct batch_number
        let request = ProofData::batch_request(self.commit_hash.clone());
        let response = connect_to_prover_server_wr(&self.prover_server_endpoint, &request)
            .await
            .map_err(|e| format!("Failed to get Response: {e}"))?;

        let (batch_number, input) = match response {
            ProofData::BatchResponse {
                batch_number,
                input,
            } => (batch_number, input),
            ProofData::InvalidCodeVersion { commit_hash } => {
                return Err(format!(
                    "Invalid code version received. Server commit_hash: {}, Prover commit_hash: {}",
                    commit_hash, self.commit_hash
                ));
            }
            _ => return Err("Expecting ProofData::Response".to_owned()),
        };

        let (Some(batch_number), Some(input)) = (batch_number, input) else {
            warn!(
                "Received Empty Response, meaning that the ProverServer doesn't have batches to prove.\nThe Prover may be advancing faster than the Proposer."
            );
            return Ok(None);
        };

        info!("Received Response for batch_number: {batch_number}");
        Ok(Some(ProverData {
            batch_number,
            input: ProgramInput {
                blocks: input.blocks,
                db: input.db,
                elasticity_multiplier: input.elasticity_multiplier,
                #[cfg(feature = "l2")]
                blob_commitment: input.blob_commitment,
                #[cfg(feature = "l2")]
                blob_proof: input.blob_proof,
            },
        }))
    }

    async fn submit_proof(&self, batch_number: u64, batch_proof: BatchProof) -> Result<(), String> {
        let submit = ProofData::proof_submit(batch_number, batch_proof);

        let ProofData::ProofSubmitACK { batch_number } =
            connect_to_prover_server_wr(&self.prover_server_endpoint, &submit)
                .await
                .map_err(|e| format!("Failed to get SubmitAck: {e}"))?
        else {
            return Err("Expecting ProofData::SubmitAck".to_owned());
        };

        info!("Received submit ack for batch_number: {batch_number}");
        Ok(())
    }
}

async fn connect_to_prover_server_wr(
    addr: &str,
    write: &ProofData,
) -> Result<ProofData, Box<dyn std::error::Error>> {
    debug!("Connecting with {addr}");
    let mut stream = TcpStream::connect(addr).await?;
    debug!("Connection established!");

    stream.write_all(&serde_json::to_vec(&write)?).await?;
    stream.shutdown().await?;

    let mut buffer = Vec::new();
    stream.read_to_end(&mut buffer).await?;

    let response: Result<ProofData, _> = serde_json::from_slice(&buffer);
    Ok(response?)
}
