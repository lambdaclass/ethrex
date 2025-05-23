use std::sync::Arc;

use crate::SequencerConfig;
use block_producer::start_block_producer;
use ethrex_blockchain::Blockchain;
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use execution_cache::ExecutionCache;
use tokio::{sync::Mutex, task::JoinSet};
use tracing::{error, info};

pub mod block_producer;
pub mod l1_committer;
pub mod l1_proof_sender;
pub mod l1_watcher;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod proof_coordinator;
pub mod state_diff;

pub mod block_fetcher;
pub mod state_updater;

pub mod execution_cache;

pub mod configs;
pub mod errors;
pub mod setup;
pub mod utils;

pub async fn start_l2(
    store: Store,
    rollup_store: StoreRollup,
    blockchain: Arc<Blockchain>,
    cfg: SequencerConfig,
) {
    let initial_state = SequencerState::default();

    info!("Starting Sequencer in {initial_state} mode");

    let shared_state = Arc::new(Mutex::new(initial_state));

    let execution_cache = Arc::new(ExecutionCache::default());

    let mut task_set = JoinSet::new();
    task_set.spawn(l1_watcher::start_l1_watcher(
        store.clone(),
        blockchain.clone(),
        cfg.clone(),
        shared_state.clone(),
    ));
    task_set.spawn(l1_committer::start_l1_committer(
        store.clone(),
        rollup_store.clone(),
        execution_cache.clone(),
        cfg.clone(),
        shared_state.clone(),
    ));
    task_set.spawn(proof_coordinator::start_proof_coordinator(
        store.clone(),
        rollup_store.clone(),
        cfg.clone(),
    ));
    task_set.spawn(l1_proof_sender::start_l1_proof_sender(
        cfg.clone(),
        shared_state.clone(),
    ));
    task_set.spawn(start_block_producer(
        store.clone(),
        blockchain.clone(),
        execution_cache.clone(),
        cfg.clone(),
        shared_state.clone(),
    ));
    task_set.spawn(state_updater::start_state_updater(
        cfg.clone(),
        shared_state.clone(),
    ));
    task_set.spawn(block_fetcher::start_block_fetcher(
        store.clone(),
        blockchain,
        execution_cache,
        shared_state.clone(),
        rollup_store,
        cfg.clone(),
    ));
    #[cfg(feature = "metrics")]
    task_set.spawn(metrics::start_metrics_gatherer(cfg));

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

#[derive(Debug, Default, Clone)]
pub enum SequencerState {
    Sequencing,
    #[default]
    Following,
}

impl std::fmt::Display for SequencerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SequencerState::Sequencing => write!(f, "Sequencing"),
            SequencerState::Following => write!(f, "Following"),
        }
    }
}
