use std::sync::Arc;

use crate::based::sequencer_state::SequencerStatus;
use crate::{BlockFetcher, SequencerConfig, StateUpdater};
use crate::{based::sequencer_state::SequencerState, utils::prover::proving_systems::ProverType};
use block_producer::BlockProducer;
use ethrex_blockchain::Blockchain;
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use execution_cache::ExecutionCache;
use l1_committer::L1Committer;
use l1_proof_sender::L1ProofSender;
use l1_watcher::L1Watcher;
use proof_coordinator::ProofCoordinator;
use tokio::task::JoinSet;
use tracing::{error, info};
use utils::get_needed_proof_types;

pub mod block_producer;
pub mod l1_committer;
pub mod l1_proof_sender;
pub mod l1_proof_verifier;
mod l1_watcher;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod proof_coordinator;

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
    let initial_status = if cfg.based.based {
        SequencerStatus::default()
    } else {
        SequencerStatus::Sequencing
    };

    info!("Starting Sequencer in {initial_status} mode");

    let shared_state = SequencerState::from(initial_status);

    let execution_cache = Arc::new(ExecutionCache::default());

    let Ok(needed_proof_types) = get_needed_proof_types(
        cfg.proof_coordinator.dev_mode,
        cfg.eth.rpc_url.clone(),
        cfg.l1_committer.on_chain_proposer_address,
    )
    .await
    .inspect_err(|e| error!("Error starting Proposer: {e}")) else {
        return;
    };

    if needed_proof_types.contains(&ProverType::Aligned) && !cfg.aligned.aligned_mode {
        error!(
            "Aligned mode is required. Please set the `--aligned` flag or use the `ALIGNED_MODE` environment variable to true."
        );
        return;
    }

    let _ = L1Watcher::spawn(
        store.clone(),
        blockchain.clone(),
        cfg.clone(),
        shared_state.clone(),
    )
    .await
    .inspect_err(|err| {
        error!("Error starting Watcher: {err}");
    });
    let _ = L1Committer::spawn(
        store.clone(),
        rollup_store.clone(),
        execution_cache.clone(),
        cfg.clone(),
        shared_state.clone(),
    )
    .await
    .inspect_err(|err| {
        error!("Error starting Committer: {err}");
    });
    let _ = ProofCoordinator::spawn(
        store.clone(),
        rollup_store.clone(),
        cfg.clone(),
        blockchain.clone(),
        needed_proof_types.clone(),
    )
    .await
    .inspect_err(|err| {
        error!("Error starting Proof Coordinator: {err}");
    });

    let _ = L1ProofSender::spawn(
        cfg.clone(),
        shared_state.clone(),
        rollup_store.clone(),
        needed_proof_types.clone(),
    )
    .await
    .inspect_err(|err| {
        error!("Error starting L1 Proof Sender: {err}");
    });
    let _ = BlockProducer::spawn(
        store.clone(),
        blockchain.clone(),
        execution_cache.clone(),
        cfg.clone(),
        shared_state.clone(),
    )
    .await
    .inspect_err(|err| {
        error!("Error starting Block Producer: {err}");
    });

    let mut task_set: JoinSet<Result<(), errors::SequencerError>> = JoinSet::new();
    if needed_proof_types.contains(&ProverType::Aligned) {
        task_set.spawn(l1_proof_verifier::start_l1_proof_verifier(cfg.clone()));
    }
    if cfg.based.based {
        let _ = StateUpdater::spawn(
            cfg.clone(),
            shared_state.clone(),
            store.clone(),
            rollup_store.clone(),
        )
        .await
        .inspect_err(|err| {
            error!("Error starting State Updater: {err}");
        });

        let _ = BlockFetcher::spawn(
            &cfg,
            store.clone(),
            rollup_store.clone(),
            blockchain.clone(),
            shared_state.clone(),
        )
        .await
        .inspect_err(|err| {
            error!("Error starting Block Fetcher: {err}");
        });
    }
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
