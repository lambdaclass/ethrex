use crate::utils::config::{proposer::ProposerConfig, read_env_file};
use errors::ProposerError;
use ethereum_rust_dev::utils::engine_client::{config::EngineApiConfig, EngineClient};
use ethereum_rust_rpc::types::fork_choice::{ForkChoiceState, PayloadAttributesV3};
use ethereum_rust_storage::Store;
use ethereum_types::{Address, H256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;
use tracing::{error, info, warn};

pub mod l1_committer;
pub mod l1_watcher;
pub mod prover_server;
pub mod state_diff;

pub mod errors;

pub struct Proposer {
    engine_client: EngineClient,
    block_production_interval: Duration,
}

pub async fn start_proposer(store: Store) {
    info!("Starting Proposer");

    if let Err(e) = read_env_file() {
        warn!("Failed to read .env file: {e}");
    }

    let l1_watcher = tokio::spawn(l1_watcher::start_l1_watcher(store.clone()));
    let l1_committer = tokio::spawn(l1_committer::start_l1_commiter(store.clone()));
    let prover_server = tokio::spawn(prover_server::start_prover_server(store.clone()));
    let proposer = tokio::spawn(async move {
        let proposer_config = ProposerConfig::from_env().expect("ProposerConfig::from_env");
        let engine_config = EngineApiConfig::from_env().expect("EngineApiConfig::from_env");
        let proposer = Proposer::new_from_config(&proposer_config, engine_config)
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
            .start(head_block_hash)
            .await
            .expect("Proposer::start");
    });
    tokio::try_join!(l1_watcher, l1_committer, prover_server, proposer).expect("tokio::try_join");
}

impl Proposer {
    pub fn new_from_config(
        proposer_config: &ProposerConfig,
        engine_config: EngineApiConfig,
    ) -> Result<Self, ProposerError> {
        Ok(Self {
            engine_client: EngineClient::new_from_config(engine_config)?,
            block_production_interval: Duration::from_millis(proposer_config.interval_ms),
        })
    }

    pub async fn start(&self, head_block_hash: H256) -> Result<(), ProposerError> {
        let mut head_block_hash = head_block_hash;
        loop {
            head_block_hash = self.produce_block(head_block_hash).await?;

            // TODO: Check what happens with the transactions included in the payload of the failed block.
            if head_block_hash == H256::zero() {
                error!("Failed to produce block");
                continue;
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
}
