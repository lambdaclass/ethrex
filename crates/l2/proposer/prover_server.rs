use crate::utils::eth_client::RpcResponse;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sp1_sdk::SP1ProofWithPublicValues;
use std::{
    io::{BufReader, BufWriter},
    net::{IpAddr, Shutdown, TcpListener, TcpStream},
    sync::mpsc::{self, Receiver},
};
use tokio::signal::unix::{signal, SignalKind};
use tracing::{debug, info, warn};

use crate::utils::config::prover_server::ProverServerConfig;

use super::errors::ProofDataProviderError;

pub async fn start_prover_server() {
    let config = ProverServerConfig::from_env().expect("ProofDataProviderConfig::from_env()");
    let prover_server = ProofDataProvider::new_from_config(config.clone());

    let (tx, rx) = mpsc::channel();

    let server = tokio::spawn(async move {
        prover_server
            .start(rx)
            .await
            .expect("prover_server.start()")
    });

    ProofDataProvider::handle_sigint(tx, config).await;

    tokio::try_join!(server).expect("tokio::try_join!()");
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ProofData {
    Request {},
    Response {
        id: Option<u64>,
    },
    Submit {
        id: u64,
        proof: Box<SP1ProofWithPublicValues>,
    },
    SubmitAck {
        id: u64,
    },
}

struct ProofDataProvider {
    ip: IpAddr,
    port: u16,
}

impl ProofDataProvider {
    pub fn new_from_config(config: ProverServerConfig) -> Self {
        Self {
            ip: config.listen_ip,
            port: config.listen_port,
        }
    }

    async fn handle_sigint(tx: mpsc::Sender<()>, config: ProverServerConfig) {
        let mut sigint = signal(SignalKind::interrupt()).expect("Failed to create SIGINT stream");
        sigint.recv().await.expect("signal.recv()");
        tx.send(()).expect("Failed to send shutdown signal");
        TcpStream::connect(format!("{}:{}", config.listen_ip, config.listen_port))
            .expect("TcpStream::connect()")
            .shutdown(Shutdown::Both)
            .expect("TcpStream::shutdown()");
    }

    pub async fn start(&self, rx: Receiver<()>) -> Result<(), ProofDataProviderError> {
        let listener = TcpListener::bind(format!("{}:{}", self.ip, self.port))?;

        let mut last_proved_block = 0;

        info!("Starting TCP server at {}:{}", self.ip, self.port);
        for stream in listener.incoming() {
            if let Ok(()) = rx.try_recv() {
                info!("Shutting down ProofDataProvider server");
                break;
            }

            debug!("Connection established!");
            self.handle_connection(stream?, &mut last_proved_block)
                .await;
        }
        Ok(())
    }

    async fn handle_connection(&self, mut stream: TcpStream, last_proved_block: &mut u64) {
        let buf_reader = BufReader::new(&stream);

        let data: Result<ProofData, _> = serde_json::de::from_reader(buf_reader);
        match data {
            Ok(ProofData::Request {}) => {
                if let Err(e) = self.handle_request(&mut stream, *last_proved_block).await {
                    warn!("Failed to handle request: {e}");
                }
            }
            Ok(ProofData::Submit { id, proof }) => {
                if let Err(e) = self.handle_submit(&mut stream, id, proof) {
                    warn!("Failed to handle submit: {e}");
                }
                *last_proved_block += 1;
            }
            Err(e) => {
                warn!("Failed to parse request: {e}");
            }
            _ => {
                warn!("Invalid request");
            }
        }

        debug!("Connection closed");
    }

    async fn get_last_block_number() -> Result<u64, String> {
        let response = Client::new()
            .post("http://localhost:8551")
            .header("content-type", "application/json")
            .body(
                r#"{
                    "jsonrpc": "2.0",
                    "method": "eth_blockNumber",
                    "params": [],
                    "id": 1
                }"#,
            )
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json::<RpcResponse>()
            .await
            .map_err(|e| e.to_string())?;

        if let RpcResponse::Success(r) = response {
            u64::from_str_radix(
                r.result
                    .as_str()
                    .ok_or("Response format error".to_string())?
                    .strip_prefix("0x")
                    .ok_or("Response format error".to_string())?,
                16,
            )
            .map_err(|e| e.to_string())
        } else {
            Err("Failed to get last block number".to_string())
        }
    }

    async fn handle_request(
        &self,
        stream: &mut TcpStream,
        last_proved_block: u64,
    ) -> Result<(), String> {
        debug!("Request received");

        let last_block_number = Self::get_last_block_number().await?;

        let response = if last_block_number > last_proved_block {
            ProofData::Response {
                id: Some(last_proved_block + 1),
            }
        } else {
            ProofData::Response { id: None }
        };
        let writer = BufWriter::new(stream);
        serde_json::to_writer(writer, &response).map_err(|e| e.to_string())
    }

    fn handle_submit(
        &self,
        stream: &mut TcpStream,
        id: u64,
        proof: Box<SP1ProofWithPublicValues>,
    ) -> Result<(), String> {
        debug!("Submit received. ID: {id}, proof: {:?}", proof.proof);

        let response = ProofData::SubmitAck { id };
        let writer = BufWriter::new(stream);
        serde_json::to_writer(writer, &response).map_err(|e| e.to_string())
    }
}
