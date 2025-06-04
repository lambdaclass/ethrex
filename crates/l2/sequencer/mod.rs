use std::sync::Arc;

use crate::SequencerConfig;
use block_producer::start_block_producer;
use ethrex_blockchain::Blockchain;
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use execution_cache::ExecutionCache;
use l1_committer::L1Committer;
use l1_watcher::L1Watcher;
use tokio::task::JoinSet;
use tracing::{error, info};

pub mod block_producer;
mod l1_committer;
pub mod l1_proof_sender;
mod l1_watcher;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod proof_coordinator;
pub mod state_diff;

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
    #[cfg(feature = "metrics")] l2_url: String,
) {
    info!("Starting Proposer");

    let execution_cache = Arc::new(ExecutionCache::default());

    L1Watcher::spawn(store.clone(), blockchain.clone(), cfg.clone()).await;
    if let Err(err) = L1Committer::spawn(
        store.clone(),
        rollup_store.clone(),
        execution_cache.clone(),
        cfg.clone(),
    )
    .await
    {
        error!("Error starting Committer: {err}");
    };

    let mut task_set = JoinSet::new();
    task_set.spawn(proof_coordinator::start_proof_coordinator(
        store.clone(),
        rollup_store.clone(),
        cfg.clone(),
        blockchain.clone(),
    ));
    task_set.spawn(l1_proof_sender::start_l1_proof_sender(cfg.clone()));
    task_set.spawn(start_block_producer(
        store.clone(),
        blockchain,
        execution_cache,
        cfg.clone(),
    ));
    #[cfg(feature = "metrics")]
    task_set.spawn(metrics::start_metrics_gatherer(cfg, rollup_store, l2_url));

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
