use crate::utils::save_prover_state::{self, *};
use ethrex_core::{
    types::{Block, BlockHeader, EIP1559Transaction},
    Address, H256,
};
use ethrex_storage::Store;
use ethrex_vm::{execution_db::ExecutionDB, EvmError};
use keccak_hash::keccak;
use secp256k1::SecretKey;
use serde::{Deserialize, Serialize};
use std::{
    io::{BufReader, BufWriter},
    net::{IpAddr, Shutdown, TcpListener, TcpStream},
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};
use tokio::{
    signal::unix::{signal, SignalKind},
    time::sleep,
};
use tracing::{debug, error, info, warn};

use risc0_zkvm::sha::{Digest, Digestible};

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ProverInputData {
    pub db: ExecutionDB,
    pub block: Block,
    pub parent_header: BlockHeader,
}

use crate::utils::{
    config::{committer::CommitterConfig, eth::EthConfig, prover_server::ProverServerConfig},
    eth_client::{errors::EthClientError, eth_sender::Overrides, EthClient},
};

use super::errors::ProverServerError;

pub async fn start_prover_server(store: Store) {
    let server_config = ProverServerConfig::from_env().expect("ProverServerConfig::from_env()");
    let eth_config = EthConfig::from_env().expect("EthConfig::from_env()");
    let proposer_config = CommitterConfig::from_env().expect("CommitterConfig::from_env()");

    if server_config.dev_mode {
        let eth_client = EthClient::new_from_config(eth_config);
        loop {
            thread::sleep(Duration::from_millis(proposer_config.interval_ms));

            let last_committed_block = EthClient::get_last_committed_block(
                &eth_client,
                proposer_config.on_chain_proposer_address,
            )
            .await
            .expect("dev_mode::get_last_committed_block()");

            let last_verified_block = EthClient::get_last_verified_block(
                &eth_client,
                proposer_config.on_chain_proposer_address,
            )
            .await
            .expect("dev_mode::get_last_verified_block()");

            if last_committed_block == u64::MAX {
                debug!("No blocks commited yet");
                continue;
            }

            if last_committed_block == last_verified_block {
                debug!("No new blocks to prove");
                continue;
            }

            info!("Last committed: {last_committed_block} - Last verified: {last_verified_block}");

            // IOnChainProposer
            // function verify(uint256,bytes,bytes32,bytes32)
            // blockNumber, seal, imageId, journalDigest
            // From crates/l2/contracts/l1/interfaces/IOnChainProposer.sol
            let mut calldata = keccak(b"verify(uint256,bytes,bytes32,bytes32)")
                .as_bytes()
                .get(..4)
                .expect("Failed to get initialize selector")
                .to_vec();
            calldata.extend(H256::from_low_u64_be(last_verified_block + 1).as_bytes());
            calldata.extend(H256::from_low_u64_be(128).as_bytes());
            calldata.extend(H256::zero().as_bytes());
            calldata.extend(H256::zero().as_bytes());
            calldata.extend(H256::zero().as_bytes());
            calldata.extend(H256::zero().as_bytes());
            let verify_tx = eth_client
                .build_eip1559_transaction(
                    proposer_config.on_chain_proposer_address,
                    calldata.into(),
                    Overrides {
                        from: Some(server_config.verifier_address),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();

            let tx_hash = eth_client
                .send_eip1559_transaction(verify_tx, &server_config.verifier_private_key)
                .await
                .unwrap();

            info!("Sending verify transaction with tx hash: {tx_hash:#x}");

            let mut retries = 1;
            while eth_client
                .get_transaction_receipt(tx_hash)
                .await
                .unwrap()
                .is_none()
            {
                thread::sleep(Duration::from_secs(1));
                retries += 1;
                if retries > 10 {
                    error!("Couldn't find receipt for transaction {tx_hash:#x}");
                    panic!("Couldn't find receipt for transaction {tx_hash:#x}");
                }
            }

            info!(
                "Mocked verify transaction sent for block {}",
                last_verified_block + 1
            );
        }
    } else {
        let mut prover_server = ProverServer::new_from_config(
            server_config.clone(),
            &proposer_config,
            eth_config,
            store,
        )
        .await
        .expect("ProverServer::new_from_config");

        let (tx, rx) = mpsc::channel();

        let server = tokio::spawn(async move {
            prover_server
                .start(rx)
                .await
                .expect("prover_server.start()")
        });

        ProverServer::handle_sigint(tx, server_config).await;

        tokio::try_join!(server).expect("tokio::try_join!()");
    }
}

/// Enum for the ProverServer <--> ProverClient Communication Protocol.
#[derive(Debug, Serialize, Deserialize)]
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
    /// for the specified block.
    Submit {
        block_number: u64,
        // zk Proof
        receipt: Box<(risc0_zkvm::Receipt, Vec<u32>)>,
    },

    /// 4.
    /// The Server acknowledges the receipt of the proof and updates its state,
    SubmitAck { block_number: u64 },
}

struct ProverServer {
    ip: IpAddr,
    port: u16,
    store: Store,
    eth_client: EthClient,
    on_chain_proposer_address: Address,
    verifier_address: Address,
    verifier_private_key: SecretKey,
}

impl ProverServer {
    pub async fn new_from_config(
        config: ProverServerConfig,
        committer_config: &CommitterConfig,
        eth_config: EthConfig,
        store: Store,
    ) -> Result<Self, EthClientError> {
        let eth_client = EthClient::new(&eth_config.rpc_url);
        let on_chain_proposer_address = committer_config.on_chain_proposer_address;

        Ok(Self {
            ip: config.listen_ip,
            port: config.listen_port,
            store,
            eth_client,
            on_chain_proposer_address,
            verifier_address: config.verifier_address,
            verifier_private_key: config.verifier_private_key,
        })
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

    pub async fn start(&mut self, rx: Receiver<()>) -> Result<(), ProverServerError> {
        let listener = TcpListener::bind(format!("{}:{}", self.ip, self.port))?;

        info!("Starting TCP server at {}:{}", self.ip, self.port);
        for stream in listener.incoming() {
            if let Ok(()) = rx.try_recv() {
                info!("Shutting down Prover Server");
                break;
            }

            debug!("Connection established!");
            self.handle_connection(stream?).await?;
        }
        Ok(())
    }

    async fn handle_connection(&mut self, mut stream: TcpStream) -> Result<(), ProverServerError> {
        // Get latestVerfiedBlock
        let last_verified_block =
            EthClient::get_last_verified_block(&self.eth_client, self.on_chain_proposer_address)
                .await?;
        let last_verified_block = if last_verified_block == u64::MAX {
            0
        } else {
            last_verified_block
        };

        let block_to_verify = last_verified_block + 1;

        let buf_reader = BufReader::new(&stream);

        let data: Result<ProofData, _> = serde_json::de::from_reader(buf_reader);
        match data {
            Ok(ProofData::Request) => {
                if let Err(e) = self.handle_request(&mut stream, block_to_verify).await {
                    warn!("Failed to handle request: {e}");
                }
            }
            Ok(ProofData::Submit {
                block_number,
                receipt,
            }) => {
                // Save Proof
                save_prover_state::write_state_in_block_state_path(
                    block_number,
                    StateType::Proof(receipt.clone()),
                    StateFileType::Proof,
                )
                .map_err(|e| {
                    ProverServerError::FailedToSaveState(
                        format!("Failed to write proof: {e:?}").to_owned(),
                    )
                })?;

                let _ = self
                    .handle_proof_submission(block_number, receipt.clone())
                    .await;

                if let Err(e) = self.handle_submit(&mut stream, block_number) {
                    error!("Failed to handle submit_ack: {e}");
                    panic!("Failed to handle submit_ack: {e}");
                }

                assert!(block_number == (block_to_verify), "Prover Client submitted an invalid block_number: {block_number}. The last_proved_block is: {}", block_to_verify);
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
        block_to_verify: u64,
    ) -> Result<(), ProverServerError> {
        debug!("Request received");
        // How to Handle shut-downs?
        // 1. Read latest proof from file
        // 2. Check if we have the proof of the block_to_verify == latestVerifiedBlock + 1
        //   - if we don't have it -> handle request and send inputs to the prover_client
        //   - if we have it -> resend tx with the same proof_number bumping the gas

        let proof: Option<Box<(risc0_zkvm::Receipt, Vec<u32>)>>;
        let has_proof;
        match save_prover_state::block_number_has_state_file(StateFileType::Proof, block_to_verify)
        {
            Ok(bool) => {
                if bool {
                    proof = match save_prover_state::read_state_file_for_block_number(
                        block_to_verify,
                        StateFileType::Proof,
                    ) {
                        Ok(state) => match state {
                            StateType::Proof(p) => {
                                has_proof = true;
                                Some(p)
                            }
                            _ => unreachable!("Expected Proof state type, but got something else"),
                        },
                        Err(e) => {
                            error!(
                                "Error reading state file for block number {}: {}",
                                block_to_verify, e,
                            );
                            // If we reach here, we may have encoded the proof wrong.
                            // Or it is missing.
                            // How can we recover this error?
                            has_proof = false;
                            None
                        }
                    };
                } else {
                    proof = None;
                    has_proof = false
                }
            }
            // Send inputs to the prover
            Err(_) => {
                proof = None;
                has_proof = false
            }
        };

        // Send a transaction with the saved proof.
        if let Some(pending_proof) = proof {
            info!("Resending saved proof {block_to_verify:?}.");
            self.handle_proof_submission(block_to_verify, pending_proof)
                .await?;
        }

        let send_input = !has_proof;
        let last_committed_block =
            EthClient::get_last_committed_block(&self.eth_client, self.on_chain_proposer_address)
                .await?;

        // Send inputs to the prover only if the last_committed_block (that comes from the OnChainProposer contract)
        // is greater or equal to the next block_number.
        // Since the block_number passed to the function is the lastVerifiedBlock, we are essentially checking if the
        // block was committed before starting the proving process.
        // Also checks if there is a proof saved.
        let response = if (last_committed_block < block_to_verify) && !send_input {
            let response = ProofData::Response {
                block_number: None,
                input: None,
            };
            warn!("Didn't send response");
            response
        } else {
            let input = self.create_prover_input(block_to_verify)?;
            let response = ProofData::Response {
                block_number: Some(block_to_verify),
                input: Some(input),
            };
            info!("Sent Response for block_number: {block_to_verify}");
            response
        };

        let writer = BufWriter::new(stream);
        serde_json::to_writer(writer, &response)
            .map_err(|e| ProverServerError::WriteError(format!("handle_request: {e}")))
    }

    fn handle_submit(
        &self,
        stream: &mut TcpStream,
        block_number: u64,
    ) -> Result<(), ProverServerError> {
        debug!("Submit received for BlockNumber: {block_number}");

        let response = ProofData::SubmitAck { block_number };
        let writer = BufWriter::new(stream);
        serde_json::to_writer(writer, &response)
            .map_err(|e| ProverServerError::WriteError(format!("handle_submit: {e}")))
    }

    async fn handle_proof_submission(
        &self,
        block_number: u64,
        receipt: Box<(risc0_zkvm::Receipt, Vec<u32>)>,
    ) -> Result<(), ProverServerError> {
        // Send Tx
        // If we run the prover_client with RISC0_DEV_MODE=0 we will have a groth16 proof
        // Else, we will have a fake proof.
        //
        // The RISC0_DEV_MODE=1 should only be used with DEPLOYER_CONTRACT_VERIFIER=0xAA
        let seal = match receipt.0.inner.groth16() {
            Ok(inner) => {
                // The SELECTOR is used to perform an extra check inside the groth16 verifier contract.
                let mut selector =
                    hex::encode(inner.verifier_parameters.as_bytes().get(..4).unwrap());
                let seal = hex::encode(inner.clone().seal);
                selector.push_str(&seal);
                hex::decode(selector).unwrap()
            }
            Err(_) => vec![32; 0],
        };

        let mut image_id: [u32; 8] = [0; 8];
        for (i, b) in image_id.iter_mut().enumerate() {
            *b = *receipt.1.get(i).unwrap();
        }

        let image_id: risc0_zkvm::sha::Digest = image_id.into();

        let journal_digest = Digestible::digest(&receipt.0.journal);

        let mut eip1559 = match self
            .build_verify_tx(block_number, &seal, image_id, journal_digest)
            .await
        {
            Ok(tx) => tx,
            Err(error) => {
                error!("Failed when constructing the verify for block {block_number:#x}. Manual intervention required: {error}");
                // TODO: retry instead of throwing error.
                // it may fail in the estimate_gas fn.
                return Err(error);
            }
        };

        // The `verify` function in the `OnChainProposer` contract has two `require` checks:
        //      "OnChainProposer: block not committed"
        //      "OnChainProposer: block already verified"
        //
        // We should never encounter the "block not committed" error, as it's handled
        // by the check: `last_committed_block < block_number`.
        //
        // If we get the "block already verified" error, it means we are submitting a `block_number`
        // smaller than the `lastVerifiedBlock` tracked by the contract, which is stored in `ProverServer::last_verified_block`.
        // We shouldn't encounter this error either.
        //
        // Both errors will trigger an error, along with any errors resulting from sending the transactions.
        let mut retries = 0;
        let max_retries: u32 = 100;
        while retries < max_retries {
            match self.send_proof(&eip1559).await {
                Ok(tx_hash) => {
                    info!(
                        "Sent proof for block {block_number}, with transaction hash {tx_hash:#x}"
                    );
                    // Remove the state path.
                    // We don't store the account updates atm.
                    // Handle errors, a retry may not be sufficient.
                    save_prover_state::delete_state_path_for_block_number(block_number).map_err(
                        |e| {
                            ProverServerError::FailedToSaveState(
                                format!("Failed to delete proof: {e:?}").to_owned(),
                            )
                        },
                    )?;
                    break;
                }

                Err(e) => {
                    error!("Failed to send proof to block {block_number:#x}. Retrying [{retries}/{max_retries}]. Error: {e}");

                    self.eth_client.bump_eip1559(&mut eip1559, 1.1);
                    retries += 1;
                    sleep(Duration::from_secs(2)).await;
                }
            }
        }

        Ok(())
    }

    fn create_prover_input(&self, block_number: u64) -> Result<ProverInputData, ProverServerError> {
        let header = self.store.get_block_header(block_number)?.ok_or(
            ProverServerError::ItemNotFoundInStore(format!(
                "block header not found for {block_number}"
            )),
        )?;
        let body = self.store.get_block_body(block_number)?.ok_or(
            ProverServerError::ItemNotFoundInStore(format!(
                "block body not found for {block_number}"
            )),
        )?;

        let block = Block::new(header, body);

        let db = ExecutionDB::from_exec(&block, &self.store).map_err(EvmError::from)?;

        let parent_header = self
            .store
            .get_block_header_by_hash(block.header.parent_hash)?
            .ok_or(ProverServerError::ItemNotFoundInStore(format!(
                "missing parent header for {block_number}"
            )))?;

        debug!("Created prover input for block {block_number}");

        Ok(ProverInputData {
            db,
            block,
            parent_header,
        })
    }

    pub async fn build_verify_tx(
        &self,
        block_number: u64,
        seal: &[u8],
        image_id: Digest,
        journal_digest: Digest,
    ) -> Result<EIP1559Transaction, ProverServerError> {
        debug!("Preparing proof");
        let mut calldata = Vec::new();

        // IOnChainProposer
        // function verify(uint256,bytes,bytes32,bytes32)
        // Verifier
        // function verify(bytes,bytes32,bytes32)
        // blockNumber, seal, imageId, journalDigest
        // From crates/l2/contracts/l1/interfaces/IOnChainProposer.sol
        let verify_proof_selector = keccak(b"verify(uint256,bytes,bytes32,bytes32)")
            .as_bytes()
            .get(..4)
            .expect("Failed to get initialize selector")
            .to_vec();
        calldata.extend(verify_proof_selector);

        // The calldata has to be structured in the following way:
        // block_number
        // size in bytes
        // image_id digest
        // journal digest
        // size of seal
        // seal

        // extend with block_number
        calldata.extend(H256::from_low_u64_be(block_number).as_bytes());

        // extend with size in bytes
        // 4 u256 goes after this field so: 0x80 == 128bytes == 32bytes * 4
        calldata.extend(H256::from_low_u64_be(4 * 32).as_bytes());

        // extend with image_id
        calldata.extend(image_id.as_bytes());

        // extend with journal_digest
        calldata.extend(journal_digest.as_bytes());

        // extend with size of seal
        calldata.extend(H256::from_low_u64_be(seal.len() as u64).as_bytes());
        // extend with seal
        calldata.extend(seal);
        // extend with zero padding
        let leading_zeros = 32 - ((calldata.len() - 4) % 32);
        calldata.extend(vec![0; leading_zeros]);

        let verify_tx = self
            .eth_client
            .build_eip1559_transaction(
                self.on_chain_proposer_address,
                calldata.into(),
                Overrides {
                    from: Some(self.verifier_address),
                    ..Default::default()
                },
            )
            .await?;

        Ok(verify_tx)
    }

    pub async fn send_proof(
        &self,
        eip1559: &EIP1559Transaction,
    ) -> Result<H256, ProverServerError> {
        let verify_tx_hash = self
            .eth_client
            .send_eip1559_transaction(eip1559.clone(), &self.verifier_private_key)
            .await?;

        eip1559_transaction_handler(
            &self.eth_client,
            eip1559,
            &self.verifier_private_key,
            verify_tx_hash,
            20,
        )
        .await?;

        Ok(verify_tx_hash)
    }
}

async fn eip1559_transaction_handler(
    eth_client: &EthClient,
    eip1559: &EIP1559Transaction,
    l1_private_key: &SecretKey,
    verify_tx_hash: H256,
    max_retries: u32,
) -> Result<H256, ProverServerError> {
    let mut retries = 0;
    let max_receipt_retries: u32 = 60 * 2; // 2 minutes
    let mut verify_tx_hash = verify_tx_hash;
    let mut tx = eip1559.clone();

    while retries < max_retries {
        if (eth_client.get_transaction_receipt(verify_tx_hash).await?).is_some() {
            // If the tx_receipt was found, return the tx_hash.
            return Ok(verify_tx_hash);
        } else {
            // Else, wait for receipt and send again if necessary.
            let mut receipt_retries = 0;

            // Try for 2 minutes with an interval of 1 second to get the tx_receipt.
            while receipt_retries < max_receipt_retries {
                match eth_client.get_transaction_receipt(verify_tx_hash).await? {
                    Some(_) => return Ok(verify_tx_hash),
                    None => {
                        receipt_retries += 1;
                        sleep(Duration::from_secs(1)).await;
                    }
                }
            }

            warn!("Transaction not confirmed, resending with 110% more gas...");
            verify_tx_hash = eth_client
                .bump_and_resend_eip1559(&mut tx, l1_private_key)
                .await?;

            retries += 1;
        }
    }
    Err(ProverServerError::FailedToVerifyProofOnChain(
        "Error handling eip1559".to_owned(),
    ))
}
