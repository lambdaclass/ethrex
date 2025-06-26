use crate::{config::ProverConfig, prove, to_batch_proof};
use ethrex_l2::{
    sequencer::proof_coordinator::{ProofData, get_code_version},
    utils::prover::proving_systems::BatchProof,
};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::sleep,
};
use tracing::{debug, error, info, warn};
use zkvm_interface::io::ProgramInput;

pub async fn start_prover(config: ProverConfig) {
    let prover_worker = Prover::new(config);
    prover_worker.start().await;
}

struct ProverData {
    batch_number: u64,
    input: ProgramInput,
}

struct Prover {
    prover_server_endpoint: String,
    proving_time_ms: u64,
    aligned_mode: bool,
    code_version: String,
}

impl Prover {
    pub fn new(cfg: ProverConfig) -> Self {
        Self {
            prover_server_endpoint: format!("{}:{}", cfg.http_addr, cfg.http_port),
            proving_time_ms: cfg.proving_time_ms,
            aligned_mode: cfg.aligned_mode,
            code_version: get_code_version(),
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
                .inspect_err(|e| warn!("Failed to request new data: {e}"))
            else {
                continue;
            };
            // If we get the input
            // Generate the Proof
            let Ok(batch_proof) = prove(prover_data.input, self.aligned_mode)
                .and_then(|output| to_batch_proof(output, self.aligned_mode))
                .inspect_err(|e| error!(e))
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

    async fn request_new_input(&self) -> Result<ProverData, String> {
        // Request the input with the correct batch_number
        let request = ProofData::batch_request(self.code_version.clone());
        let response = connect_to_prover_server_wr(&self.prover_server_endpoint, &request)
            .await
            .map_err(|e| format!("Failed to get Response: {e}"))?;

        let (batch_number, input) = match response {
            ProofData::BatchResponse {
                batch_number,
                input,
            } => (batch_number, input),
            ProofData::InvalidCodeVersion { code_version } => {
                return Err(fmt!(
                    "Invalid code version received. Server code: {}, Prover code: {}",
                    code_version,
                    self.code_version
                ));
            }
            _ => return Err("Expecting ProofData::Response".to_owned()),
        };

        let (Some(batch_number), Some(input)) = (batch_number, input) else {
            return Err(
                    "Received Empty Response, meaning that the ProverServer doesn't have batches to prove.\nThe Prover may be advancing faster than the Proposer."
                        .to_owned(),
                );
        };

        info!("Received Response for batch_number: {batch_number}");
        Ok(ProverData {
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
        })
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
