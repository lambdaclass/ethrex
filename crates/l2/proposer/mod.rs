use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::utils::config::{errors::ConfigError, proposer::ProposerConfig, read_env_file};
use errors::ProposerError;
use ethereum_types::Address;
use ethrex_blockchain::{
    error::ChainError,
    find_parent_header,
    payload::{create_payload, BuildPayloadArgs},
    store_block, store_receipts, validate_block, validate_gas_used, validate_receipts_root,
    validate_state_root, Blockchain,
};
use ethrex_rpc::clients::EngineApiConfig;
use ethrex_storage::Store;
use keccak_hash::H256;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tracing::{debug, error, info};

pub mod l1_committer;
pub mod l1_watcher;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod prover_server;
pub mod state_diff;

pub mod errors;

pub struct Proposer {
    interval_ms: u64,
    coinbase_address: Address,
}

pub async fn start_proposer(store: Store, blockchain: Arc<Blockchain>) {
    info!("Starting Proposer");

    if let Err(e) = read_env_file() {
        error!("Failed to read .env file: {e}");
        return;
    }

    let mut task_set = JoinSet::new();
    task_set.spawn(l1_watcher::start_l1_watcher(
        store.clone(),
        blockchain.clone(),
    ));
    task_set.spawn(l1_committer::start_l1_committer(store.clone()));
    task_set.spawn(prover_server::start_prover_server(store.clone()));
    task_set.spawn(start_proposer_server(store.clone(), blockchain));
    #[cfg(feature = "metrics")]
    task_set.spawn(metrics::start_metrics_gatherer());

    while let Some(res) = task_set.join_next().await {
        match res {
            Ok(Ok(_)) => {}
            Ok(Err(err)) => {
                error!("Error starting Proposer: {err}");
                task_set.abort_all();
                break;
            }
            Err(err) => {
                error!("JoinSet error: {err}");
                task_set.abort_all();
                break;
            }
        };
    }
}

async fn start_proposer_server(
    store: Store,
    blockchain: Arc<Blockchain>,
) -> Result<(), ConfigError> {
    let proposer_config = ProposerConfig::from_env()?;
    let proposer = Proposer::new_from_config(proposer_config).map_err(ConfigError::from)?;

    proposer.run(store.clone(), blockchain).await;
    Ok(())
}

impl Proposer {
    pub fn new_from_config(config: ProposerConfig) -> Result<Self, ProposerError> {
        let ProposerConfig {
            interval_ms,
            coinbase_address,
        } = config;
        Ok(Self {
            interval_ms,
            coinbase_address,
        })
    }

    pub async fn run(&self, store: Store, blockchain: Arc<Blockchain>) {
        loop {
            if let Err(err) = self.main_logic(store.clone(), blockchain.clone()).await {
                error!("Block Producer Error: {}", err);
            }

            sleep(Duration::from_millis(self.interval_ms)).await;
        }
    }

    pub async fn main_logic(
        &self,
        store: Store,
        blockchain: Arc<Blockchain>,
    ) -> Result<(), ProposerError> {
        let version = 3;
        let head_block_hash = {
            let current_block_number = store.get_latest_block_number()?;
            store
                .get_canonical_block_hash(current_block_number)?
                .ok_or(ProposerError::StorageDataIsNone)?
        };
        let parent_beacon_block_root = H256::zero();

        // ethrex_dev::block_producer::start_block_producer(
        //     self.engine_config.rpc_url.clone(),
        //     self.jwt_secret.clone().into(),
        //     head_block_hash,
        //     10,
        //     self.block_production_interval,
        //     self.coinbase_address,
        // )
        // .await?;

        // The proposer leverages the execution payload framework used for the engine API,
        // but avoids calling the API methods and unnecesary re-execution.

        info!("Producing block");
        debug!("Head block hash: {head_block_hash:#x}");

        // Create payload
        let args = BuildPayloadArgs {
            parent: head_block_hash,
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            fee_recipient: self.coinbase_address,
            random: H256::zero(),
            withdrawals: Default::default(),
            beacon_root: Some(parent_beacon_block_root),
            version,
        };
        let mut payload = create_payload(&args, &store)?;
        // let payload_id = ...

        // Build payload and executes transactions
        let (.., mut evm_state, block_cache, receipts) = blockchain.build_payload(&mut payload)?;
        let account_updates =
            blockchain
                .vm
                .get_state_transitions(&mut evm_state, head_block_hash, &block_cache)?;

        // Add to state
        // TODO: validation
        //store.add_payload(payload_id, block)

        let block = payload;
        let Ok(parent_header) = find_parent_header(&block.header, &store) else {
            // If the parent is not present, we store it as pending.
            store.add_pending_block(block.clone())?;
            return Err(ProposerError::ChainError(ChainError::ParentNotFound));
        };
        let chain_config = evm_state
            .chain_config()
            .map_err(ChainError::from)
            .map_err(ProposerError::from)?;
        validate_block(&block, &parent_header, &chain_config)?;

        validate_gas_used(&receipts, &block.header)?;

        // Apply the account updates over the last block's state and compute the new state root
        let new_state_root = evm_state
            .database()
            .unwrap()
            //.ok_or(ProposerError::StoreError(StoreError::MissingStore))?
            .apply_account_updates(block.header.parent_hash, &account_updates)?
            .unwrap();
        //.ok_or(ChainError::ParentStateNotFound)?;

        // Check state root matches the one in block header after execution
        validate_state_root(&block.header, new_state_root)?;

        // Check receipts root matches the one in block header after execution
        validate_receipts_root(&block.header, &receipts)?;

        // Processes requests from receipts, computes the requests_hash and compares it against the header
        //validate_requests_hash(&block.header, &chain_config, &requests)?;

        store_block(&store, block.clone())?;
        store_receipts(&store, receipts, block.header.compute_block_hash())?;

        Ok(())
    }
}
