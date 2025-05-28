use crate::{config::ProverConfig, prove, to_calldata};
use ethrex_l2::{
    sequencer::proof_coordinator::ProofData, utils::prover::proving_systems::ProofCalldata,
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
}

impl Prover {
    pub fn new(config: ProverConfig) -> Self {
        Self {
            prover_server_endpoint: config.prover_server_endpoint,
            proving_time_ms: config.proving_time_ms,
        }
    }

    pub async fn start(&self) {
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
            let Ok(proving_output) = prove(prover_data.input)
                .and_then(to_calldata)
                .inspect_err(|e| error!(e))
            else {
                continue;
            };

            let _ = self
                .submit_proof(prover_data.batch_number, proving_output)
                .await
                .inspect_err(|e|
                    // TODO: Retry?
                    warn!("Failed to submit proof: {e}"));
        }
    }

    async fn request_new_input(&self) -> Result<ProverData, String> {
        // Request the input with the correct batch_number
        let request = ProofData::batch_request();
        let response = connect_to_prover_server_wr(&self.prover_server_endpoint, &request)
            .await
            .map_err(|e| format!("Failed to get Response: {e}"))?;

        let ProofData::BatchResponse {
            batch_number,
            input,
        } = response
        else {
            return Err("Expecting ProofData::Response".to_owned());
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
                parent_block_header: input.parent_block_header,
                db: input.db,
                elasticity_multiplier: input.elasticity_multiplier,
            },
        })
    }

    async fn submit_proof(
        &self,
        batch_number: u64,
        proving_output: ProofCalldata,
    ) -> Result<(), String> {
        let submit = ProofData::proof_submit(batch_number, proving_output);

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
    debug!("Got response {}", hex::encode(&buffer));

    let response: Result<ProofData, _> = serde_json::from_slice(&buffer);
    Ok(response?)
}
