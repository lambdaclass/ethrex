use crate::utils::eth_client::RpcResponse;
use ethereum_rust_storage::Store;
use ethereum_rust_vm::execution_db::ExecutionDB;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{
    io::{BufReader, BufWriter},
    net::{IpAddr, Shutdown, TcpListener, TcpStream},
    sync::mpsc::{self, Receiver},
};
use tokio::signal::unix::{signal, SignalKind};
use tracing::{debug, info, warn};

use ethereum_rust_core::types::{Block, BlockHeader};

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ProverInputData {
    pub db: ExecutionDB,
    pub block: Block,
    pub parent_header: BlockHeader,
}

use crate::utils::config::prover_server::ProverServerConfig;

use super::errors::ProverServerError;

pub async fn start_prover_server(store: Store) {
    let config = ProverServerConfig::from_env().expect("ProverServerConfig::from_env()");
    let prover_server = ProverServer::new_from_config(config.clone(), store);

    let (tx, rx) = mpsc::channel();

    let server = tokio::spawn(async move {
        prover_server
            .start(rx)
            .await
            .expect("prover_server.start()")
    });

    ProverServer::handle_sigint(tx, config).await;

    tokio::try_join!(server).expect("tokio::try_join!()");
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ProofData {
    Request {},
    Response {
        block_number: Option<u64>,
        input: ProverInputData,
    },
    Submit {
        block_number: u64,
        // zk Proof
        receipt: Box<risc0_zkvm::Receipt>,
    },
    SubmitAck {
        block_number: u64,
    },
}

struct ProverServer {
    ip: IpAddr,
    port: u16,
    store: Store,
}

impl ProverServer {
    pub fn new_from_config(config: ProverServerConfig, store: Store) -> Self {
        Self {
            ip: config.listen_ip,
            port: config.listen_port,
            store,
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

    pub async fn start(&self, rx: Receiver<()>) -> Result<(), ProverServerError> {
        let listener = TcpListener::bind(format!("{}:{}", self.ip, self.port))?;

        let mut last_proved_block = 0;

        info!("Starting TCP server at {}:{}", self.ip, self.port);
        for stream in listener.incoming() {
            if let Ok(()) = rx.try_recv() {
                info!("Shutting down Prover Server");
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
            Ok(ProofData::Submit {
                block_number,
                receipt,
            }) => {
                if let Err(e) = self.handle_submit(&mut stream, block_number, receipt) {
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

    async fn _get_last_block_number() -> Result<u64, String> {
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

        let last_block_number = self
            .store
            .get_latest_block_number()
            .map_err(|e| e.to_string())?
            .ok_or("missing latest block number".to_string())?;
        let input = self.create_prover_input(last_block_number)?;

        let response = if last_block_number > last_proved_block {
            ProofData::Response {
                block_number: Some(last_block_number),
                input,
            }
        } else {
            ProofData::Response {
                block_number: None,
                input,
            }
        };
        let writer = BufWriter::new(stream);
        serde_json::to_writer(writer, &response).map_err(|e| e.to_string())
    }

    fn handle_submit(
        &self,
        stream: &mut TcpStream,
        block_number: u64,
        receipt: Box<risc0_zkvm::Receipt>,
    ) -> Result<(), String> {
        debug!("Submit received. ID: {block_number}, proof: {:?}", receipt);

        let response = ProofData::SubmitAck { block_number };
        let writer = BufWriter::new(stream);
        serde_json::to_writer(writer, &response).map_err(|e| e.to_string())
    }

    fn create_prover_input(&self, block_number: u64) -> Result<ProverInputData, String> {
        let header = self
            .store
            .get_block_header(block_number)
            .map_err(|err| err.to_string())?
            .ok_or("block header not found")?;
        let body = self
            .store
            .get_block_body(block_number)
            .map_err(|err| err.to_string())?
            .ok_or("block body not found")?;

        let block = Block::new(header, body);

        let db = ExecutionDB::from_exec(&block, &self.store).map_err(|err| err.to_string())?;

        let parent_header = self
            .store
            .get_block_header_by_hash(block.header.parent_hash)
            .map_err(|err| err.to_string())?
            .ok_or("missing parent header".to_string())?;

        debug!("Created prover input for block {block_number}");

        Ok(ProverInputData {
            db,
            block,
            parent_header,
        })
    }
}
