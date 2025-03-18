use std::sync::Arc;

use crate::utils::config::read_env_file;
use block_producer::start_block_producer;
use ethrex_blockchain::Blockchain;
use ethrex_storage::Store;
use execution_cache::ExecutionCache;
use tokio::task::JoinSet;
use tracing::{error, info};

pub mod block_producer;
pub mod l1_committer;
pub mod l1_watcher;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod prover_server;
pub mod state_diff;

pub mod execution_cache;

pub mod errors;

pub async fn start_l2(store: Store, blockchain: Arc<Blockchain>) {
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
    task_set.spawn(start_block_producer(
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
