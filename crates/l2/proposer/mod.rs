use std::{
    sync::Arc,
    thread::current,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::utils::config::{errors::ConfigError, proposer::ProposerConfig, read_env_file};
use errors::ProposerError;
use ethereum_types::Address;
use ethrex_blockchain::{
    error::ChainError,
    find_parent_header,
    payload::{create_payload, BuildPayloadArgs, PayloadBuildResult},
    store_block, store_receipts, validate_block, validate_gas_used, validate_receipts_root,
    validate_state_root, Blockchain,
};
use ethrex_rpc::clients::EngineApiConfig;
use ethrex_storage::Store;
use ethrex_vm::backends::BlockExecutionResult;
use execution_cache::ExecutionCache;
use keccak_hash::H256;
use tokio::time::sleep;
use tokio::{sync::broadcast, task::JoinSet};
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
    execution_cache: ExecutionCache,
}

pub async fn start_proposer(store: Store, blockchain: Arc<Blockchain>) {
    info!("Starting Proposer");

    if let Err(e) = read_env_file() {
        error!("Failed to read .env file: {e}");
        return;
    }

    const EXECUTION_CACHE_LEN: usize = 16;
    let execution_cache = ExecutionCache::new(EXECUTION_CACHE_LEN);

    let mut task_set = JoinSet::new();
    task_set.spawn(l1_watcher::start_l1_watcher(
        store.clone(),
        blockchain.clone(),
    ));
    task_set.spawn(l1_committer::start_l1_committer(
        store.clone(),
        execution_cache.subscribe(),
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
    execution_cache: ExecutionCache,
) -> Result<(), ConfigError> {
    let proposer_config = ProposerConfig::from_env()?;
    let proposer =
        Proposer::new_from_config(proposer_config, execution_cache).map_err(ConfigError::from)?;

    proposer.run(store.clone(), blockchain).await;
    Ok(())
}

impl Proposer {
    pub fn new_from_config(
        config: ProposerConfig,
        execution_cache: ExecutionCache,
    ) -> Result<Self, ProposerError> {
        let ProposerConfig {
            interval_ms,
            coinbase_address,
        } = config;
        Ok(Self {
            interval_ms,
            coinbase_address,
            execution_cache,
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
        let parent_header = {
            let current_block_number = store.get_latest_block_number()?;
            store
                .get_block_header(current_block_number)?
                .ok_or(ProposerError::StorageDataIsNone)?
        };
        let parent_hash = parent_header.compute_block_hash();
        let parent_beacon_block_root = H256::zero();

        // The proposer leverages the execution payload framework used for the engine API,
        // but avoids calling the API methods and unnecesary re-execution.

        info!("Producing block");
        debug!("Head block hash: {parent_hash:#x}");

        // Proposer creates a new payload
        let args = BuildPayloadArgs {
            parent: parent_hash,
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            fee_recipient: self.coinbase_address,
            random: H256::zero(),
            withdrawals: Default::default(),
            beacon_root: Some(parent_beacon_block_root),
            version,
        };
        let mut payload = create_payload(&args, &store)?;

        // Blockchain builds the payload from mempool txs and executes them
        let mut payload_build_result = blockchain.build_payload(&mut payload)?;
        let account_updates =
            payload_build_result.get_state_transitions(parent_hash, blockchain.vm)?;
        info!("Built payload for new block {}", payload.hash());

        // Blockchain stores block
        let block = payload;
        let chain_config = store.get_chain_config()?;
        validate_block(&block, &parent_header, &chain_config)?;

        let execution_result = BlockExecutionResult {
            account_updates,
            receipts: payload_build_result.receipts,
            requests: Vec::new(),
        };

        blockchain.store_block(&block, execution_result.clone())?;
        info!("Stored new block {}", block.hash());

        // Cache execution result
        self.execution_cache
            .push(block.header.number, execution_result);

        // WARN: We're not storing the payload into the Store because there's no use to it by the L2 for now.

        Ok(())
    }
}
