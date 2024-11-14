use std::{
    io::{BufReader, BufWriter},
    net::TcpStream,
    time::Duration,
};

use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use ethereum_rust_l2::{
    proposer::prover_server::{ProofData, ProverInputData},
    utils::config::prover_client::ProverClientConfig,
};

use super::prover::Prover;

pub async fn start_proof_data_client(config: ProverClientConfig) {
    let proof_data_client = ProverClient::new(config);
    proof_data_client.start().await;
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
        let mut prover = Prover::new();

        loop {
            match self.request_new_input() {
                Ok((block_number, input)) => {
                    match prover.set_input(input).prove() {
                        Ok(proof) => {
                            if let Err(e) =
                                self.submit_proof(block_number, proof, prover.id.to_vec())
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
        }
    }

    fn request_new_input(&self) -> Result<(u64, ProverInputData), String> {
        // Request the input with the correct block_number
        let request = ProofData::Request;
        let response = connect_to_prover_server_wr(&self.prover_server_endpoint, &request)
            .map_err(|e| format!("Failed to get Response: {e}"))?;

        match response {
            ProofData::Response {
                block_number,
                input,
            } => match (block_number, input) {
                (Some(n), Some(i)) => {
                    info!("Received Response for block_number: {n}");
                    Ok((n, i))
                }
                _ => Err(
                    "Received Empty Response, meaning that the ProverServer doesn't have blocks to prove.\nThe Prover may be advancing faster than the Proposer."
                        .to_owned(),
                ),
            },
            _ => Err(format!("Expecting ProofData::Response  {response:?}")),
        }
    }

    fn submit_proof(
        &self,
        block_number: u64,
        receipt: risc0_zkvm::Receipt,
        prover_id: Vec<u32>,
    ) -> Result<(), String> {
        let submit = ProofData::Submit {
            block_number,
            receipt: Box::new((receipt, prover_id)),
        };
        let submit_ack = connect_to_prover_server_wr(&self.prover_server_endpoint, &submit)
            .map_err(|e| format!("Failed to get SubmitAck: {e}"))?;

        match submit_ack {
            ProofData::SubmitAck { block_number } => {
                info!("Received submit ack for block_number: {}", block_number);
                Ok(())
            }
            _ => Err(format!("Expecting ProofData::SubmitAck {submit_ack:?}")),
        }
    }
}

fn connect_to_prover_server_wr(
    addr: &str,
    write: &ProofData,
) -> Result<ProofData, Box<dyn std::error::Error>> {
    let stream = TcpStream::connect(addr)?;
    let buf_writer = BufWriter::new(&stream);
    debug!("Connection established!");
    serde_json::ser::to_writer(buf_writer, write)?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let buf_reader = BufReader::new(&stream);
    let response: ProofData = serde_json::de::from_reader(buf_reader)?;
    Ok(response)
}
