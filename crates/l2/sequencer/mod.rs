use std::sync::Arc;

use crate::SequencerConfig;
use block_producer::start_block_producer;
use errors::{CommitterError, L1WatcherError};
use ethrex_blockchain::Blockchain;
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use execution_cache::ExecutionCache;
use l1_committer::Committer;
use l1_watcher::L1Watcher;
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

pub mod execution_cache;

pub mod configs;
pub mod errors;
pub mod utils;

pub async fn start_l2(
    store: Store,
    rollup_store: StoreRollup,
    blockchain: Arc<Blockchain>,
    cfg: SequencerConfig,
) {
    info!("Starting Proposer");

    let state = Arc::new(Mutex::new(SequencerState::default()));

    let execution_cache = Arc::new(ExecutionCache::default());

    let mut task_set = JoinSet::new();
    task_set.spawn(l1_watcher::start_l1_watcher(
        store.clone(),
        blockchain.clone(),
        cfg.clone(),
        state.clone(),
    ));
    task_set.spawn(l1_committer::start_l1_committer(
        store.clone(),
        rollup_store.clone(),
        execution_cache.clone(),
        cfg.clone(),
        state.clone(),
    ));
    task_set.spawn(proof_coordinator::start_proof_coordinator(
        store.clone(),
        rollup_store,
        cfg.clone(),
    ));
    task_set.spawn(l1_proof_sender::start_l1_proof_sender(cfg.clone()));
    task_set.spawn(start_block_producer(
        store.clone(),
        blockchain,
        execution_cache,
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

#[derive(Debug, Default)]
pub enum SequencerState {
    /// The sequencer builds blocks, commits batches, and sends proofs to L1.
    #[default]
    Sequencing,
    /// The node is syncing the L2 chain from L1.
    Syncing,
}

impl SequencerState {
    pub async fn commit_batch(
        &self,
        committer: &mut Committer,
    ) -> Result<SequencerState, CommitterError> {
        match self {
            SequencerState::Sequencing => {
                committer.commit_batch().await?;
                Ok(SequencerState::Sequencing)
            }
            SequencerState::Syncing => Ok(SequencerState::Syncing),
        }
    }

    pub async fn watch(
        &self,
        watcher: &mut L1Watcher,
        store: &Store,
        blockchain: &Blockchain,
    ) -> Result<(), L1WatcherError> {
        match self {
            SequencerState::Sequencing => {
                watcher.watch(store, blockchain).await?;
            }
            SequencerState::Syncing => {}
        };
        Ok(())
    }
}
