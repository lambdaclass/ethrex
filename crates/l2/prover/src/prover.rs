use crate::{backend::Backend, config::ProverConfig, prove, to_batch_proof};
use ethrex_l2::sequencer::proof_coordinator::{ProofData, get_commit_hash};
use ethrex_l2_common::prover::BatchProof;
use guest_program::input::ProgramInput;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::sleep,
};
use tracing::{debug, error, info, warn};
use url::Url;

pub async fn start_prover(config: ProverConfig) {
    let prover_worker = Prover::new(config);
    prover_worker.start().await;
}

struct ProverData {
    batch_number: u64,
    input: ProgramInput,
}

struct Prover {
    backend: Backend,
    proof_coordinator_endpoints: Vec<Url>,
    proving_time_ms: u64,
    aligned_mode: bool,
    commit_hash: String,
}

impl Prover {
    pub fn new(cfg: ProverConfig) -> Self {
        Self {
            backend: cfg.backend,
            proof_coordinator_endpoints: cfg.proof_coordinators,
            proving_time_ms: cfg.proving_time_ms,
            aligned_mode: cfg.aligned_mode,
            commit_hash: get_commit_hash(),
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
        // Build the prover depending on the prover_type passed as argument.
        loop {
            sleep(Duration::from_millis(self.proving_time_ms)).await;

            for endpoint in &self.proof_coordinator_endpoints {
                let Ok(Some(prover_data)) = self
                    .request_new_input(endpoint)
                    .await
                    .inspect_err(|e| error!(%endpoint, "Failed to request new data from: {e}"))
                else {
                    continue;
                };

                // If we get the input
                // Generate the Proof
                let Ok(batch_proof) = prove(self.backend, prover_data.input, self.aligned_mode)
                    .and_then(|output| to_batch_proof(output, self.aligned_mode))
                    .inspect_err(|e| error!(%endpoint, "{}", e.to_string()))
                else {
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

    async fn request_new_input(&self, endpoint: &Url) -> Result<Option<ProverData>, String> {
        // Request the input with the correct batch_number
        let request = ProofData::batch_request(self.commit_hash.clone());
        let response = connect_to_prover_server_wr(endpoint, &request)
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
                %endpoint,
                "Received Empty Response, meaning that the ProverServer doesn't have batches to prove.\nThe Prover may be advancing faster than the Proposer."
            );
            return Ok(None);
        };

        info!(%endpoint, "Received Response for batch_number: {batch_number}");
        Ok(Some(ProverData {
            batch_number,
            input: ProgramInput {
                blocks: input.blocks,
                execution_witness: input.execution_witness,
                elasticity_multiplier: input.elasticity_multiplier,
                #[cfg(feature = "l2")]
                blob_commitment: input.blob_commitment,
                #[cfg(feature = "l2")]
                blob_proof: input.blob_proof,
                fee_config: Some(input.fee_config),
            },
        }))
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
