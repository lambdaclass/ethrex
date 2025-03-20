use crate::{prove, to_calldata};
use ethrex_l2::{
    sequencer::prover_server::ProofData,
    utils::{config::prover_client::ProverClientConfig, prover::proving_systems::ProofCalldata},
};
use std::{
    io::{BufReader, BufWriter},
    time::Duration,
};
use tokio::{io::AsyncReadExt, net::TcpStream};
use tokio::{io::AsyncWriteExt, time::sleep};
use tracing::{debug, error, info, warn};
use zkvm_interface::io::ProgramInput;

pub async fn start_proof_data_client(config: ProverClientConfig) {
    let proof_data_client = ProverClient::new(config);
    proof_data_client.start().await;
}

struct ProverData {
    block_number: u64,
    input: ProgramInput,
}

struct ProverClient {
    prover_server_endpoint: String,
    interval_ms: u64,
}

impl ProverClient {
    pub fn new(config: ProverClientConfig) -> Self {
        Self {
            prover_server_endpoint: config.prover_server_endpoint,
            interval_ms: config.interval_ms,
        }
    }

    pub async fn start(&self) {
        // Build the prover depending on the prover_type passed as argument.
        loop {
            match self.request_new_input().await {
                // If we get the input
                Ok(prover_data) => {
                    // Generate the Proof
                    match prove(prover_data.input).and_then(to_calldata) {
                        Ok(proving_output) => {
                            if let Err(e) = self
                                .submit_proof(prover_data.block_number, proving_output)
                                .await
                            {
                                // TODO: Retry?
                                warn!("Failed to submit proof: {e}");
                            }
                        }
                        Err(e) => error!(e),
                    };
                }
                Err(e) => {
                    sleep(Duration::from_millis(self.interval_ms)).await;
                    warn!("Failed to request new data: {e}");
                }
            }
            sleep(Duration::from_millis(self.interval_ms)).await;
        }
    }

    async fn request_new_input(&self) -> Result<ProverData, String> {
        // Request the input with the correct block_number
        let request = ProofData::request();
        let response = connect_to_prover_server_wr(&self.prover_server_endpoint, &request)
            .await
            .map_err(|e| format!("Failed to get Response: {e}"))?;

        match response {
            ProofData::Response {
                block_number,
                input,
            } => match (block_number, input) {
                (Some(block_number), Some(input)) => {
                    info!("Received Response for block_number: {block_number}");
                    let prover_data = ProverData{
                        block_number,
                        input:  ProgramInput {
                            block: input.block,
                            parent_block_header: input.parent_block_header,
                            db: input.db
                        }
                    };
                    Ok(prover_data)
                }
                _ => Err(
                    "Received Empty Response, meaning that the ProverServer doesn't have blocks to prove.\nThe Prover may be advancing faster than the Proposer."
                        .to_owned(),
                ),
            },
            _ => Err("Expecting ProofData::Response".to_owned()),
        }
    }

    async fn submit_proof(
        &self,
        block_number: u64,
        proving_output: ProofCalldata,
    ) -> Result<(), String> {
        let submit = ProofData::submit(block_number, proving_output);

        let submit_ack = connect_to_prover_server_wr(&self.prover_server_endpoint, &submit)
            .await
            .map_err(|e| format!("Failed to get SubmitAck: {e}"))?;

        match submit_ack {
            ProofData::SubmitAck { block_number } => {
                info!("Received submit ack for block_number: {}", block_number);
                Ok(())
            }
            _ => Err("Expecting ProofData::SubmitAck".to_owned()),
        }
    }
}

async fn connect_to_prover_server_wr(
    addr: &str,
    write: &ProofData,
) -> Result<ProofData, Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect(addr).await?;
    debug!("Connection established!");

    stream.write(&serde_json::to_vec(&write)?).await?;
    stream.shutdown().await?;

    let mut buffer = Vec::new();
    stream.read_to_end(&mut buffer).await?;

    let response: Result<ProofData, _> = serde_json::from_slice(&buffer);
    Ok(response?)
}
