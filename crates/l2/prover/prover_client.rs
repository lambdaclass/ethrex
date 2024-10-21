use std::{
    io::{BufReader, BufWriter},
    net::TcpStream,
    time::Duration,
};

use sp1_sdk::SP1ProofWithPublicValues;
use tokio::time::sleep;
use tracing::{debug, error, warn};

use crate::{proposer::prover_server::ProofData, utils::config::prover_client::ProverClientConfig};

use super::prover::Prover;

pub async fn start_prover_client() {
    let config = ProverClientConfig::from_env().unwrap();
    let prover_client = ProofDataClient::new(config.prover_server_endpoint.clone());
    prover_client.start(config).await;
}

struct ProofDataClient {
    prover_server_endpoint: String,
}

impl ProofDataClient {
    pub fn new(prover_server_endpoint: String) -> Self {
        Self {
            prover_server_endpoint,
        }
    }

    pub async fn start(&self, config: ProverClientConfig) {
        let prover = Prover::new_from_config(config);

        loop {
            match self.request_new_data() {
                Ok(Some(id)) => {
                    match prover.prove(id) {
                        Ok(proof) => {
                            if let Err(e) = self.submit_proof(id, proof) {
                                // TODO: Retry
                                warn!("Failed to submit proof: {e}");
                            }
                        }
                        Err(e) => error!(e),
                    };
                }
                Ok(None) => sleep(Duration::from_secs(10)).await,
                Err(e) => warn!("Failed to request new data: {e}"),
            }
        }
    }

    fn request_new_data(&self) -> Result<Option<u64>, String> {
        let stream = TcpStream::connect(&self.prover_server_endpoint)
            .map_err(|e| format!("Failed to connect to ProofDataProvider: {e}"))?;
        let buf_writer = BufWriter::new(&stream);

        debug!("Connection established!");

        let request = ProofData::Request {};
        serde_json::ser::to_writer(buf_writer, &request).map_err(|e| e.to_string())?;
        stream
            .shutdown(std::net::Shutdown::Write)
            .map_err(|e| e.to_string())?;

        let buf_reader = BufReader::new(&stream);
        let response: ProofData = serde_json::de::from_reader(buf_reader)
            .map_err(|e| format!("Invalid response format: {e}"))?;

        match response {
            ProofData::Response { id } => {
                debug!("Received response: {id:?}");
                Ok(id)
            }
            _ => Err(format!("Unexpected response {response:?}")),
        }
    }

    fn submit_proof(&self, id: u64, proof: SP1ProofWithPublicValues) -> Result<(), String> {
        let stream = TcpStream::connect(&self.prover_server_endpoint)
            .map_err(|e| format!("Failed to connect to ProofDataProvider: {e}"))?;
        let buf_writer = BufWriter::new(&stream);

        let submit = ProofData::Submit {
            id,
            proof: Box::new(proof),
        };
        serde_json::ser::to_writer(buf_writer, &submit).map_err(|e| e.to_string())?;
        stream
            .shutdown(std::net::Shutdown::Write)
            .map_err(|e| e.to_string())?;

        let buf_reader = BufReader::new(&stream);
        let response: ProofData = serde_json::de::from_reader(buf_reader)
            .map_err(|e| format!("Invalid response format: {e}"))?;
        match response {
            ProofData::SubmitAck { id: res_id } => {
                debug!("Received submit ack: {res_id}");
                Ok(())
            }
            _ => Err(format!("Unexpected response {response:?}")),
        }
    }
}
