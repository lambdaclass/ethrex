use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::utils::config::{errors::ConfigError, proposer::ProposerConfig, read_env_file};
use errors::ProposerError;
use ethereum_types::Address;
use ethrex_blockchain::{
    fork_choice::apply_fork_choice,
    payload::{create_payload, BuildPayloadArgs},
    validate_block, Blockchain,
};
use ethrex_storage::Store;
use ethrex_vm::backends::BlockExecutionResult;
use execution_cache::ExecutionCache;
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

pub mod execution_cache;

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

    let execution_cache = Arc::new(ExecutionCache::default());

    let mut task_set = JoinSet::new();
    task_set.spawn(l1_watcher::start_l1_watcher(
        store.clone(),
        blockchain.clone(),
    ));
    task_set.spawn(l1_committer::start_l1_committer(
        store.clone(),
        execution_cache.clone(),
    ));
    task_set.spawn(prover_server::start_prover_server(store.clone()));
    task_set.spawn(start_proposer_server(
        store.clone(),
        blockchain,
        execution_cache,
    ));
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
    execution_cache: Arc<ExecutionCache>,
) -> Result<(), ConfigError> {
    let proposer_config = ProposerConfig::from_env()?;
    let proposer = Proposer::new_from_config(proposer_config).map_err(ConfigError::from)?;

    proposer
        .run(store.clone(), blockchain, execution_cache)
        .await;
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

    pub async fn run(
        &self,
        store: Store,
        blockchain: Arc<Blockchain>,
        execution_cache: Arc<ExecutionCache>,
    ) {
        loop {
            if let Err(err) =
                self.main_logic(store.clone(), blockchain.clone(), execution_cache.clone())
            {
                error!("Block Producer Error: {}", err);
            }

            sleep(Duration::from_millis(self.interval_ms)).await;
        }
    }

    pub fn main_logic(
        &self,
        store: Store,
        blockchain: Arc<Blockchain>,
        execution_cache: Arc<ExecutionCache>,
    ) -> Result<(), ProposerError> {
        let version = 3;
        let head_header = {
            let current_block_number = store.get_latest_block_number()?;
            store
                .get_block_header(current_block_number)?
                .ok_or(ProposerError::StorageDataIsNone)?
        };
        let head_hash = head_header.compute_block_hash();
        let head_beacon_block_root = H256::zero();

        // The proposer leverages the execution payload framework used for the engine API,
        // but avoids calling the API methods and unnecesary re-execution.

        info!("Producing block");
        debug!("Head block hash: {head_hash:#x}");

        // Proposer creates a new payload
        let args = BuildPayloadArgs {
            parent: head_hash,
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            fee_recipient: self.coinbase_address,
            random: H256::zero(),
            withdrawals: Default::default(),
            beacon_root: Some(head_beacon_block_root),
            version,
        };
        let mut payload = create_payload(&args, &store)?;

        // Blockchain builds the payload from mempool txs and executes them
        let payload_build_result = blockchain.build_payload(&mut payload)?;
        info!("Built payload for new block {}", payload.header.number);

        // Blockchain stores block
        let block = payload;
        let chain_config = store.get_chain_config()?;
        validate_block(&block, &head_header, &chain_config)?;

        let execution_result = BlockExecutionResult {
            account_updates: payload_build_result.account_updates,
            receipts: payload_build_result.receipts,
            requests: Vec::new(),
        };

        blockchain.store_block(&block, execution_result.clone())?;
        info!("Stored new block {:x}", block.hash());
        // WARN: We're not storing the payload into the Store because there's no use to it by the L2 for now.

        // Cache execution result
        execution_cache.push(block.hash(), execution_result.account_updates)?;

        // Make the new head be part of the canonical chain
        apply_fork_choice(&store, block.hash(), block.hash(), block.hash())?;

        Ok(())
    }
}
