use crate::sequencer::errors::{ProverServerError, SigIntError};
use crate::utils::{
    config::{
        committer::CommitterConfig, errors::ConfigError, eth::EthConfig,
        prover_server::ProverServerConfig,
    },
    prover::{
        errors::SaveStateError,
        proving_systems::{ProofCalldata, ProverType},
        save_state::{StateFileType, StateType, *},
    },
};
use ethrex_common::{
    types::{Block, BlockHeader},
    Address, H256, U256,
};
use ethrex_l2_sdk::calldata::{encode_calldata, Value};
use ethrex_rpc::clients::eth::{eth_sender::Overrides, EthClient, WrappedTransaction};
use ethrex_storage::Store;
use ethrex_vm::{
    backends::revm::execution_db::{ExecutionDB, ToExecDB},
    db::StoreWrapper,
    EvmError,
};
use secp256k1::SecretKey;
use serde::{Deserialize, Serialize};
use std::{
    fmt::Debug,
    net::IpAddr,
    sync::mpsc::{self, Receiver},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::{
    signal::unix::{signal, SignalKind},
    time::sleep,
};
use tracing::{debug, error, info, warn};

const VERIFY_FUNCTION_SIGNATURE: &str =
    "verify(uint256,bytes,bytes,bytes32,bytes32,bytes32,bytes,bytes,bytes32,bytes,uint256[8])";

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ProverInputData {
    pub block: Block,
    pub parent_block_header: BlockHeader,
    pub db: ExecutionDB,
}

#[derive(Clone)]
struct ProverServer {
    ip: IpAddr,
    port: u16,
    store: Store,
    eth_client: EthClient,
    on_chain_proposer_address: Address,
    verifier_address: Address,
    verifier_private_key: SecretKey,
    dev_interval_ms: u64,
}

/// Enum for the ProverServer <--> ProverClient Communication Protocol.
#[derive(Serialize, Deserialize)]
pub enum ProofData {
    /// 1.
    /// The Client initiates the connection with a Request.
    /// Asking for the ProverInputData the prover_server considers/needs.
    Request,

    /// 2.
    /// The Server responds with a Response containing the ProverInputData.
    /// If the Response will is ProofData::Response{None, None}, the Client knows that the Request couldn't be performed.
    Response {
        block_number: Option<u64>,
        input: Option<ProverInputData>,
    },

    /// 3.
    /// The Client submits the zk Proof generated by the prover
    /// for the specified block, as calldata for the verifier contract.
    Submit {
        block_number: u64,
        calldata: ProofCalldata,
    },

    /// 4.
    /// The Server acknowledges the receipt of the proof and updates its state,
    SubmitAck { block_number: u64 },
}

impl ProofData {
    /// Builder function for creating a Request
    pub fn request() -> Self {
        ProofData::Request
    }

    /// Builder function for creating a Response
    pub fn response(block_number: Option<u64>, input: Option<ProverInputData>) -> Self {
        ProofData::Response {
            block_number,
            input,
        }
    }

    /// Builder function for creating a Submit
    pub fn submit(block_number: u64, calldata: ProofCalldata) -> Self {
        ProofData::Submit {
            block_number,
            calldata,
        }
    }

    /// Builder function for creating a SubmitAck
    pub fn submit_ack(block_number: u64) -> Self {
        ProofData::SubmitAck { block_number }
    }
}

pub async fn start_prover_server(store: Store) -> Result<(), ConfigError> {
    let server_config = ProverServerConfig::from_env()?;
    let eth_config = EthConfig::from_env()?;
    let proposer_config = CommitterConfig::from_env()?;
    let mut prover_server =
        ProverServer::new_from_config(server_config.clone(), &proposer_config, eth_config, store)
            .await;
    prover_server.run(&server_config).await;
    Ok(())
}

impl ProverServer {
    pub async fn new_from_config(
        config: ProverServerConfig,
        committer_config: &CommitterConfig,
        eth_config: EthConfig,
        store: Store,
    ) -> Self {
        let eth_client = EthClient::new(&eth_config.rpc_url);
        let on_chain_proposer_address = committer_config.on_chain_proposer_address;

        Self {
            ip: config.listen_ip,
            port: config.listen_port,
            store,
            eth_client,
            on_chain_proposer_address,
            verifier_address: config.verifier_address,
            verifier_private_key: config.verifier_private_key,
            dev_interval_ms: config.dev_interval_ms,
        }
    }

    pub async fn run(&mut self, server_config: &ProverServerConfig) {
        loop {
            let result = if server_config.dev_mode {
                self.main_logic_dev().await
            } else {
                self.clone().main_logic(server_config).await
            };

            match result {
                Ok(_) => {
                    if !server_config.dev_mode {
                        warn!("Prover Server shutting down");
                        break;
                    }
                }
                Err(e) => {
                    let error_message = if !server_config.dev_mode {
                        format!("Prover Server, severe Error, trying to restart the main_logic function: {e}")
                    } else {
                        format!("Prover Server Dev Error: {e}")
                    };
                    error!(error_message);
                }
            }

            sleep(Duration::from_millis(200)).await;
        }
    }

    async fn main_logic(
        mut self,
        server_config: &ProverServerConfig,
    ) -> Result<(), ProverServerError> {
        let (tx, rx) = mpsc::channel();

        // It should never exit the start() fn, handling errors inside the for loop of the function.
        let server_handle = tokio::spawn(async move { self.start(rx).await });

        ProverServer::handle_sigint(tx, server_config).await?;

        match server_handle.await {
            Ok(result) => match result {
                Ok(_) => (),
                Err(e) => return Err(e),
            },
            Err(e) => return Err(e.into()),
        };

        Ok(())
    }

    async fn handle_sigint(
        tx: mpsc::Sender<()>,
        config: &ProverServerConfig,
    ) -> Result<(), ProverServerError> {
        let mut sigint = signal(SignalKind::interrupt())?;
        sigint.recv().await.ok_or(SigIntError::Recv)?;
        tx.send(()).map_err(SigIntError::Send)?;
        TcpStream::connect(format!("{}:{}", config.listen_ip, config.listen_port))
            .await?
            .shutdown()
            .await
            .map_err(SigIntError::Shutdown)?;

        Ok(())
    }

    pub async fn start(&mut self, rx: Receiver<()>) -> Result<(), ProverServerError> {
        let listener = TcpListener::bind(format!("{}:{}", self.ip, self.port)).await?;

        info!("Starting TCP server at {}:{}", self.ip, self.port);

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    debug!("Connection established!");

                    if let Ok(()) = rx.try_recv() {
                        info!("Shutting down Prover Server");
                        break;
                    }

                    if let Err(e) = self.handle_connection(stream).await {
                        error!("Error handling connection: {}", e);
                    }
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }
        Ok(())
    }

    async fn handle_connection(&mut self, mut stream: TcpStream) -> Result<(), ProverServerError> {
        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer).await?;

        let last_verified_block =
            EthClient::get_last_verified_block(&self.eth_client, self.on_chain_proposer_address)
                .await?;

        let block_to_verify = last_verified_block + 1;

        let mut tx_submitted = false;

        // If we have all the proofs send a transaction to verify them on chain

        let send_tx = match block_number_has_all_proofs(block_to_verify) {
            Ok(has_all_proofs) => has_all_proofs,
            Err(e) => {
                if let SaveStateError::IOError(ref error) = e {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        return Err(e.into());
                    }
                } else {
                    return Err(e.into());
                }
                false
            }
        };
        if send_tx {
            self.handle_proof_submission(block_to_verify).await?;
            // Remove the Proofs for that block_number
            prune_state(block_to_verify)?;
            tx_submitted = true;
        }

        let data: Result<ProofData, _> = serde_json::from_slice(&buffer);
        match data {
            Ok(ProofData::Request) => {
                if let Err(e) = self
                    .handle_request(&mut stream, block_to_verify, tx_submitted)
                    .await
                {
                    warn!("Failed to handle request: {e}");
                }
            }
            Ok(ProofData::Submit {
                block_number,
                calldata,
            }) => {
                self.handle_submit(&mut stream, block_number).await?;

                // Avoid storing a proof of a future block_number
                // CHECK: maybe we would like to store all the proofs given the case in which
                // the provers generate them fast enough. In this way, we will avoid unneeded reexecution.
                if block_number != block_to_verify {
                    return Err(ProverServerError::Custom(format!("Prover Client submitted an invalid block_number: {block_number}. The last_proved_block is: {last_verified_block}")));
                }

                // If the transaction was submitted for the block_to_verify
                // avoid storing already used proofs.
                if tx_submitted {
                    return Ok(());
                }

                // Check if we have the proof for that ProverType
                // If we don't have it, insert it.
                let has_proof = match block_number_has_state_file(
                    StateFileType::Proof(calldata.prover_type),
                    block_number,
                ) {
                    Ok(has_proof) => has_proof,
                    Err(e) => {
                        let error = format!("{e}");
                        if !error.contains("No such file or directory") {
                            return Err(e.into());
                        }
                        false
                    }
                };
                if !has_proof {
                    write_state(block_number, &StateType::Proof(calldata))?;
                }

                // Then if we have all the proofs, we send the transaction in the next `handle_connection` call.
            }
            Err(e) => {
                warn!("Failed to parse request: {e}");
            }
            _ => {
                warn!("Invalid request");
            }
        }

        debug!("Connection closed");
        Ok(())
    }

    async fn handle_request(
        &self,
        stream: &mut TcpStream,
        block_number: u64,
        tx_submitted: bool,
    ) -> Result<(), ProverServerError> {
        debug!("Request received");

        let latest_block_number = self.store.get_latest_block_number()?;

        let response = if block_number > latest_block_number {
            let response = ProofData::response(None, None);
            debug!("Didn't send response");
            response
        } else if tx_submitted {
            let response = ProofData::response(None, None);
            debug!("Block: {block_number} has been submitted.");
            response
        } else {
            let input = self.create_prover_input(block_number)?;
            let response = ProofData::response(Some(block_number), Some(input));
            info!("Sent Response for block_number: {block_number}");
            response
        };

        let buffer = serde_json::to_vec(&response)?;
        stream
            .write_all(&buffer)
            .await
            .map_err(ProverServerError::ConnectionError)?;
        Ok(())
    }

    async fn handle_submit(
        &self,
        stream: &mut TcpStream,
        block_number: u64,
    ) -> Result<(), ProverServerError> {
        debug!("Submit received for BlockNumber: {block_number}");

        let response = ProofData::submit_ack(block_number);

        let buffer = serde_json::to_vec(&response)?;
        stream
            .write_all(&buffer)
            .await
            .map_err(ProverServerError::ConnectionError)?;
        Ok(())
    }

    fn create_prover_input(&self, block_number: u64) -> Result<ProverInputData, ProverServerError> {
        let header = self
            .store
            .get_block_header(block_number)?
            .ok_or(ProverServerError::StorageDataIsNone)?;
        let body = self
            .store
            .get_block_body(block_number)?
            .ok_or(ProverServerError::StorageDataIsNone)?;

        let block = Block::new(header, body);

        let parent_hash = block.header.parent_hash;
        let store = StoreWrapper {
            store: self.store.clone(),
            block_hash: parent_hash,
        };
        let db = store.to_exec_db(&block).map_err(EvmError::ExecutionDB)?;

        let parent_block_header = self
            .store
            .get_block_header_by_hash(parent_hash)?
            .ok_or(ProverServerError::StorageDataIsNone)?;

        debug!("Created prover input for block {block_number}");

        Ok(ProverInputData {
            db,
            block,
            parent_block_header,
        })
    }

    pub async fn handle_proof_submission(
        &self,
        block_number: u64,
    ) -> Result<H256, ProverServerError> {
        // TODO change error
        let exec_proof = read_proof(block_number, StateFileType::Proof(ProverType::Exec))?;
        if exec_proof.prover_type != ProverType::Exec {
            return Err(ProverServerError::Custom(
                "Exec Proof isn't present".to_string(),
            ));
        }

        let risc0_proof = read_proof(block_number, StateFileType::Proof(ProverType::RISC0))?;
        if risc0_proof.prover_type != ProverType::RISC0 {
            return Err(ProverServerError::Custom(
                "RISC0 Proof isn't present".to_string(),
            ));
        }

        let sp1_proof = read_proof(block_number, StateFileType::Proof(ProverType::SP1))?;
        if sp1_proof.prover_type != ProverType::SP1 {
            return Err(ProverServerError::Custom(
                "SP1 Proof isn't present".to_string(),
            ));
        }

        let pico_proof = read_proof(block_number, StateFileType::Proof(ProverType::Pico))?;
        if pico_proof.prover_type != ProverType::Pico {
            return Err(ProverServerError::Custom(
                "Pico Proof isn't present".to_string(),
            ));
        }

        debug!("Sending proof for {block_number}");

        let calldata_values = [
            &[Value::Uint(U256::from(block_number))],
            exec_proof.calldata.as_slice(),
            risc0_proof.calldata.as_slice(),
            sp1_proof.calldata.as_slice(),
            pico_proof.calldata.as_slice(),
        ]
        .concat();

        warn!("calldata value len: {}", calldata_values.len());

        let calldata = encode_calldata(VERIFY_FUNCTION_SIGNATURE, &calldata_values)?;

        let verify_tx = self
            .eth_client
            .build_eip1559_transaction(
                self.on_chain_proposer_address,
                self.verifier_address,
                calldata.into(),
                Overrides::default(),
                10,
            )
            .await?;

        let verify_tx_hash = self
            .eth_client
            .send_wrapped_transaction_with_retry(
                &WrappedTransaction::EIP1559(verify_tx),
                &self.verifier_private_key,
                3 * 60,
                10,
            )
            .await?;

        info!("Sent proof for block {block_number}, with transaction hash {verify_tx_hash:#x}");

        Ok(verify_tx_hash)
    }

    pub async fn main_logic_dev(&self) -> Result<(), ProverServerError> {
        loop {
            tokio::time::sleep(Duration::from_millis(self.dev_interval_ms)).await;

            let last_committed_block = EthClient::get_last_committed_block(
                &self.eth_client,
                self.on_chain_proposer_address,
            )
            .await?;

            let last_verified_block = EthClient::get_last_verified_block(
                &self.eth_client,
                self.on_chain_proposer_address,
            )
            .await?;

            if last_committed_block == last_verified_block {
                debug!("No new blocks to prove");
                continue;
            }

            info!("Last committed: {last_committed_block} - Last verified: {last_verified_block}");

            let calldata_values = vec![
                // blockNumber
                Value::Uint(U256::from(last_verified_block + 1)),
                // execPublicInputs
                Value::Bytes(vec![].into()),
                // risc0BlockProof
                Value::Bytes(vec![].into()),
                // risc0ImageId
                Value::FixedBytes(H256::zero().as_bytes().to_vec().into()),
                // risco0JournalDigest
                Value::FixedBytes(H256::zero().as_bytes().to_vec().into()),
                // sp1ProgramVKey
                Value::FixedBytes(H256::zero().as_bytes().to_vec().into()),
                // sp1PublicValues
                Value::Bytes(vec![].into()),
                // sp1Bytes
                Value::Bytes(vec![].into()),
                // picoRiscvVkey
                Value::FixedBytes(H256::zero().as_bytes().to_vec().into()),
                // picoPublicValues
                Value::Bytes(vec![].into()),
                // picoProof
                Value::FixedArray(vec![Value::Uint(U256::zero()); 8]),
            ];

            let calldata = encode_calldata(VERIFY_FUNCTION_SIGNATURE, &calldata_values)?;

            let verify_tx = self
                .eth_client
                .build_eip1559_transaction(
                    self.on_chain_proposer_address,
                    self.verifier_address,
                    calldata.into(),
                    Overrides {
                        ..Default::default()
                    },
                    10,
                )
                .await?;

            info!("Sending verify transaction.");

            let verify_tx_hash = self
                .eth_client
                .send_wrapped_transaction_with_retry(
                    &WrappedTransaction::EIP1559(verify_tx),
                    &self.verifier_private_key,
                    3 * 60,
                    10,
                )
                .await?;

            info!("Sent proof for block {last_verified_block}, with transaction hash {verify_tx_hash:#x}");

            info!(
                "Mocked verify transaction sent for block {}",
                last_verified_block + 1
            );
        }
    }
}
