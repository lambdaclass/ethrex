use crate::utils::{
    config::{eth::EthConfig, proposer::ProposerConfig, read_env_file},
    eth_client::EthClient,
    merkle_tree::merkelize,
};
use bytes::Bytes;
use errors::ProposerError;
use ethereum_rust_blockchain::constants::TX_GAS_COST;
use ethereum_rust_core::types::{
    Block, EIP1559Transaction, GenericTransaction, PrivilegedTxType, Transaction, TxKind,
};
use ethereum_rust_dev::utils::engine_client::{config::EngineApiConfig, EngineClient};
use ethereum_rust_rlp::encode::RLPEncode;
use ethereum_rust_rpc::types::fork_choice::{ForkChoiceState, PayloadAttributesV3};
use ethereum_rust_storage::Store;
use ethereum_types::{Address, H256, U256};
use keccak_hash::keccak;
use libsecp256k1::SecretKey;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;
use tracing::{error, info, warn};

pub mod l1_watcher;
pub mod prover_server;

pub mod errors;

const COMMIT_FUNCTION_SELECTOR: [u8; 4] = [132, 97, 12, 179];
const VERIFY_FUNCTION_SELECTOR: [u8; 4] = [133, 133, 44, 228];

pub struct Proposer {
    eth_client: EthClient,
    engine_client: EngineClient,
    on_chain_proposer_address: Address,
    l1_address: Address,
    l1_private_key: SecretKey,
    block_production_interval: Duration,
}

pub async fn start_proposer(store: Store) {
    info!("Starting Proposer");

    if let Err(e) = read_env_file() {
        warn!("Failed to read .env file: {e}");
    }

    let l1_watcher = tokio::spawn(l1_watcher::start_l1_watcher(store.clone()));
    let prover_server = tokio::spawn(prover_server::start_prover_server(store.clone()));
    let proposer = tokio::spawn(async move {
        let eth_config = EthConfig::from_env().expect("EthConfig::from_env");
        let proposer_config = ProposerConfig::from_env().expect("ProposerConfig::from_env");
        let engine_config = EngineApiConfig::from_env().expect("EngineApiConfig::from_env");
        let proposer = Proposer::new_from_config(&proposer_config, eth_config, engine_config)
            .expect("Proposer::new_from_config");
        let head_block_hash = {
            let current_block_number = store
                .get_latest_block_number()
                .expect("store.get_latest_block_number")
                .expect("store.get_latest_block_number returned None");
            store
                .get_canonical_block_hash(current_block_number)
                .expect("store.get_canonical_block_hash")
                .expect("store.get_canonical_block_hash returned None")
        };
        proposer
            .start(head_block_hash, store)
            .await
            .expect("Proposer::start");
    });
    tokio::try_join!(l1_watcher, prover_server, proposer).expect("tokio::try_join");
}

impl Proposer {
    pub fn new_from_config(
        proposer_config: &ProposerConfig,
        eth_config: EthConfig,
        engine_config: EngineApiConfig,
    ) -> Result<Self, ProposerError> {
        Ok(Self {
            eth_client: EthClient::new(&eth_config.rpc_url),
            engine_client: EngineClient::new_from_config(engine_config)?,
            on_chain_proposer_address: proposer_config.on_chain_proposer_address,
            l1_address: proposer_config.l1_address,
            l1_private_key: proposer_config.l1_private_key,
            block_production_interval: Duration::from_millis(proposer_config.interval_ms),
        })
    }

    pub async fn start(&self, head_block_hash: H256, store: Store) -> Result<(), ProposerError> {
        let mut head_block_hash = head_block_hash;
        loop {
            head_block_hash = self.produce_block(head_block_hash).await?;

            // TODO: Check what happens with the transactions included in the payload of the failed block.
            if head_block_hash == H256::zero() {
                error!("Failed to produce block");
                continue;
            }

            let block = store
                .get_block_by_hash(head_block_hash)
                .map_err(|error| {
                    ProposerError::FailedToRetrieveBlockFromStorage(error.to_string())
                })?
                .ok_or(ProposerError::FailedToProduceBlock(
                    "Failed to get block by hash from storage".to_string(),
                ))?;

            let withdrawal_data_hashes: Vec<H256> = block
                .body
                .transactions
                .iter()
                .filter_map(|tx| match tx {
                    Transaction::PrivilegedL2Transaction(tx) => tx.get_withdrawal_hash(),
                    _ => None,
                })
                .collect();

            let withdrawals_logs_merkle_root = if !withdrawal_data_hashes.is_empty() {
                merkelize(withdrawal_data_hashes.clone())
            } else {
                H256::zero()
            };

            let deposit_hashes: Vec<[u8; 32]> = block
                .body
                .transactions
                .iter()
                .filter_map(|tx| match tx {
                    Transaction::PrivilegedL2Transaction(tx)
                        if tx.tx_type == PrivilegedTxType::Deposit =>
                    {
                        let to = match tx.to {
                            TxKind::Call(to) => to,
                            TxKind::Create => Address::zero(),
                        };
                        let value_bytes = &mut [0u8; 32];
                        tx.value.to_big_endian(value_bytes);
                        Some(keccak([H256::from(to).0, H256::from(value_bytes).0].concat()).0)
                    }
                    _ => None,
                })
                .collect();
            let deposit_logs_hash = if deposit_hashes.is_empty() {
                H256::zero()
            } else {
                H256::from_slice(
                    [
                        &(deposit_hashes.len() as u16).to_be_bytes(),
                        &keccak(deposit_hashes.concat()).0[2..32],
                    ]
                    .concat()
                    .as_slice(),
                )
            };

            let new_state_root_hash = store
                .state_trie(block.hash())
                .unwrap()
                .unwrap()
                .hash()
                .unwrap();

            match self
                .send_commitment(
                    block.header.number,
                    new_state_root_hash,
                    withdrawals_logs_merkle_root,
                    deposit_logs_hash,
                )
                .await
            {
                Ok(commit_tx_hash) => {
                    info!(
                    "Sent commitment to block {head_block_hash:#x}, with transaction hash {commit_tx_hash:#x}"
                );
                }
                Err(error) => {
                    error!("Failed to send commitment to block {head_block_hash:#x}. Manual intervention required: {error}");
                    panic!("Failed to send commitment to block {head_block_hash:#x}. Manual intervention required: {error}");
                }
            }

            let proof = Vec::new();

            match self.send_proof(block.header.number, &proof).await {
                Ok(verify_tx_hash) => {
                    info!(
                    "Sent proof for block {head_block_hash}, with transaction hash {verify_tx_hash:#x}"
                );
                }
                Err(error) => {
                    error!("Failed to send proof to block {head_block_hash:#x}. Manual intervention required: {error}");
                    panic!("Failed to send proof to block {head_block_hash:#x}. Manual intervention required: {error}");
                }
            }

            sleep(self.block_production_interval).await;
        }
    }

    pub async fn produce_block(&self, head_block_hash: H256) -> Result<H256, ProposerError> {
        info!("Producing block");
        let fork_choice_state = ForkChoiceState {
            head_block_hash,
            safe_block_hash: head_block_hash,
            finalized_block_hash: head_block_hash,
        };
        let payload_attributes = PayloadAttributesV3 {
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            // Setting the COINBASE address / fee_recipient.
            // TODO: revise it, maybe we would like to have this set with an envar
            suggested_fee_recipient: Address::from_slice(
                &hex::decode("0007a881CD95B1484fca47615B64803dad620C8d").unwrap(),
            ),
            ..Default::default()
        };
        let fork_choice_response = match self
            .engine_client
            .engine_forkchoice_updated_v3(fork_choice_state, Some(payload_attributes))
            .await
        {
            Ok(response) => response,
            Err(error) => {
                error!("Error sending forkchoiceUpdateV3: {error}");
                return Err(ProposerError::FailedToProduceBlock(format!(
                    "forkchoiceUpdateV3: {error}",
                )));
            }
        };
        let payload_id =
            fork_choice_response
                .payload_id
                .ok_or(ProposerError::FailedToProduceBlock(
                    "payload_id is None in ForkChoiceResponse".to_string(),
                ))?;
        let execution_payload_response =
            match self.engine_client.engine_get_payload_v3(payload_id).await {
                Ok(response) => response,
                Err(error) => {
                    error!("Error sending getPayloadV3: {error}");
                    return Err(ProposerError::FailedToProduceBlock(format!(
                        "getPayloadV3: {error}"
                    )));
                }
            };
        let payload_status = match self
            .engine_client
            .engine_new_payload_v3(
                execution_payload_response.execution_payload,
                Default::default(),
                Default::default(),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => {
                error!("Error sending newPayloadV3: {error}");
                return Err(ProposerError::FailedToProduceBlock(format!(
                    "newPayloadV3: {error}"
                )));
            }
        };
        let produced_block_hash =
            payload_status
                .latest_valid_hash
                .ok_or(ProposerError::FailedToProduceBlock(
                    "latest_valid_hash is None in PayloadStatus".to_string(),
                ))?;
        info!("Produced block {produced_block_hash:#x}");
        Ok(produced_block_hash)
    }

    pub async fn prepare_commitment(&self, block: Block) -> H256 {
        info!("Preparing commitment");
        keccak(block.encode_to_vec())
    }

    pub async fn send_commitment(
        &self,
        block_number: u64,
        new_l2_state_root: H256,
        withdrawal_logs_merkle_root: H256,
        deposit_logs_hash: H256,
    ) -> Result<H256, ProposerError> {
        info!("Sending commitment");
        let mut calldata = Vec::with_capacity(132);
        calldata.extend(COMMIT_FUNCTION_SELECTOR);
        let mut block_number_bytes = [0_u8; 32];
        U256::from(block_number).to_big_endian(&mut block_number_bytes);
        calldata.extend(block_number_bytes);
        calldata.extend(new_l2_state_root.0);
        calldata.extend(withdrawal_logs_merkle_root.0);
        calldata.extend(deposit_logs_hash.0);

        let commit_tx_hash = self
            .send_transaction_with_calldata(self.on_chain_proposer_address, calldata.into())
            .await?;

        info!("Commitment sent: {commit_tx_hash:#x}");

        while self
            .eth_client
            .get_transaction_receipt(commit_tx_hash)
            .await?
            .is_none()
        {
            sleep(Duration::from_secs(1)).await;
        }

        Ok(commit_tx_hash)
    }

    pub async fn send_proof(
        &self,
        block_number: u64,
        block_proof: &[u8],
    ) -> Result<H256, ProposerError> {
        info!("Sending proof");
        let mut calldata = Vec::new();
        calldata.extend(VERIFY_FUNCTION_SELECTOR);
        let mut block_number_bytes = [0_u8; 32];
        U256::from(block_number).to_big_endian(&mut block_number_bytes);
        calldata.extend(block_number_bytes);
        calldata.extend(H256::from_low_u64_be(32).as_bytes());
        calldata.extend(H256::from_low_u64_be(block_proof.len() as u64).as_bytes());
        calldata.extend(block_proof);
        let leading_zeros = 32 - ((calldata.len() - 4) % 32);
        calldata.extend(vec![0; leading_zeros]);

        let verify_tx_hash = self
            .send_transaction_with_calldata(self.on_chain_proposer_address, calldata.into())
            .await?;

        info!("Proof sent: {verify_tx_hash:#x}");

        while self
            .eth_client
            .get_transaction_receipt(verify_tx_hash)
            .await?
            .is_none()
        {
            sleep(Duration::from_secs(1)).await;
        }

        Ok(verify_tx_hash)
    }

    async fn send_transaction_with_calldata(
        &self,
        to: Address,
        calldata: Bytes,
    ) -> Result<H256, ProposerError> {
        let mut tx = EIP1559Transaction {
            to: TxKind::Call(to),
            data: calldata,
            max_fee_per_gas: self.eth_client.get_gas_price().await?.as_u64(),
            nonce: self.eth_client.get_nonce(self.l1_address).await?,
            chain_id: self.eth_client.get_chain_id().await?.as_u64(),
            ..Default::default()
        };

        let mut generic_tx = GenericTransaction::from(tx.clone());
        generic_tx.from = self.l1_address;

        tx.gas_limit = self
            .eth_client
            .estimate_gas(generic_tx)
            .await?
            .saturating_add(TX_GAS_COST);

        self.eth_client
            .send_eip1559_transaction(&mut tx, self.l1_private_key)
            .await
            .map_err(ProposerError::from)
    }
}
