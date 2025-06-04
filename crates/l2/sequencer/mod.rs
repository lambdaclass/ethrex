use std::sync::Arc;

use crate::SequencerConfig;
use block_producer::BlockProducer;
use ethrex_blockchain::Blockchain;
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use execution_cache::ExecutionCache;
use l1_committer::L1Committer;
use l1_proof_sender::L1ProofSender;
use l1_watcher::L1Watcher;
#[cfg(feature = "metrics")]
use metrics::MetricsGatherer;
use proof_coordinator::ProofCoordinator;
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

    let _ = L1Watcher::spawn(store.clone(), blockchain.clone(), cfg.clone())
        .await
        .inspect_err(|err| {
            error!("Error starting Watcher: {err}");
        });
    let _ = L1Committer::spawn(
        store.clone(),
        rollup_store.clone(),
        execution_cache.clone(),
        cfg.clone(),
    )
    .await
    .inspect_err(|err| {
        error!("Error starting Committer: {err}");
    });
    let _ = ProofCoordinator::spawn(store.clone(), rollup_store.clone(), cfg.clone())
        .await
        .inspect_err(|err| {
            error!("Error starting Proof Coordinator: {err}");
        });
    let _ = L1ProofSender::spawn(cfg.clone()).await.inspect_err(|err| {
        error!("Error starting Proof Coordinator: {err}");
    });
    let _ = BlockProducer::spawn(
        store.clone(),
        blockchain,
        execution_cache.clone(),
        cfg.clone(),
    )
    .await
    .inspect_err(|err| {
        error!("Error starting Block Producer: {err}");
    });

    #[cfg(feature = "metrics")]
    let _ = MetricsGatherer::spawn(cfg, rollup_store, l2_url)
        .await
        .inspect_err(|err| {
            error!("Error starting Block Producer: {err}");
        });
}
