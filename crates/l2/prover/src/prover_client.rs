use crate::prover::create_prover;
use ethrex_l2::{
    proposer::prover_server::{ProofData, ProverType, ProvingOutput},
    utils::config::prover_client::ProverClientConfig,
};
use std::{
    io::{BufReader, BufWriter},
    net::TcpStream,
    time::Duration,
};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};
use zkvm_interface::io::ProgramInput;

pub async fn start_proof_data_client(config: ProverClientConfig, prover_type: ProverType) {
    let proof_data_client = ProverClient::new(config);
    proof_data_client.start(prover_type).await;
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

    pub async fn start(&self, prover_type: ProverType) {
        // Build the prover depending on the prover_type passed as argument.
        let mut prover = create_prover(prover_type);

        loop {
            match self.request_new_input() {
                // If we get the input
                Ok(prover_data) => {
                    // Generate the Proof
                    match prover.prove(prover_data.input) {
                        Ok(proving_output) => {
                            if let Err(e) =
                                self.submit_proof(prover_data.block_number, proving_output)
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

    fn request_new_input(&self) -> Result<ProverData, String> {
        // Request the input with the correct block_number
        let request = ProofData::request();
        let response = connect_to_prover_server_wr(&self.prover_server_endpoint, &request)
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

    fn submit_proof(&self, block_number: u64, proving_output: ProvingOutput) -> Result<(), String> {
        let submit = match proving_output {
            ProvingOutput::RISC0(risc0_proof) => {
                ProofData::submit(block_number, ProvingOutput::RISC0(risc0_proof))
            }
            ProvingOutput::SP1(sp1_proof) => {
                ProofData::submit(block_number, ProvingOutput::SP1(sp1_proof))
            }
        };

        let submit_ack = connect_to_prover_server_wr(&self.prover_server_endpoint, &submit)
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
