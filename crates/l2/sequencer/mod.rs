use std::sync::Arc;

use crate::SequencerConfig;
use crate::based::{
    block_fetcher::BlockFetcher,
    sequencer_state::{SequencerState, SequencerStatus},
    state_updater::StateUpdater,
};
use crate::monitor::EthrexMonitor;
use crate::sequencer::admin_server::start_api;
use crate::sequencer::errors::SequencerError;
use block_producer::BlockProducer;
use ethrex_blockchain::Blockchain;
use ethrex_l2_common::prover::ProverType;
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use l1_committer::L1Committer;
use l1_proof_sender::L1ProofSender;
use l1_watcher::L1Watcher;
#[cfg(feature = "metrics")]
use metrics::MetricsGatherer;
use proof_coordinator::ProofCoordinator;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use utils::get_needed_proof_types;

mod admin_server;
pub mod block_producer;
pub mod l1_committer;
pub mod l1_proof_sender;
pub mod l1_proof_verifier;
pub mod l1_watcher;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod proof_coordinator;

pub mod configs;
pub mod errors;
pub mod setup;
pub mod utils;

pub async fn start_l2(
    store: Store,
    rollup_store: StoreRollup,
    blockchain: Arc<Blockchain>,
    cfg: SequencerConfig,
    cancellation_token: CancellationToken,
    #[cfg(feature = "metrics")] l2_url: String,
) -> Result<(), errors::SequencerError> {
    let initial_status = if cfg.based.enabled {
        SequencerStatus::default()
    } else {
        SequencerStatus::Sequencing
    };

    if let Some(batch_gas_limit) = cfg.l1_committer.batch_gas_limit {
        let block_gas_limit = cfg.block_producer.block_gas_limit;
        if batch_gas_limit < block_gas_limit {
            error!(
                "The block gas limit ({block_gas_limit}) cannot be greater than the batch gas limit ({batch_gas_limit})."
            );
            return Err(errors::SequencerError::GasLimitError);
        }
    }

    info!("Starting Sequencer in {initial_status} mode");

    let shared_state = SequencerState::from(initial_status);

    let Ok(needed_proof_types) = get_needed_proof_types(
        cfg.eth.rpc_url.clone(),
        cfg.l1_committer.on_chain_proposer_address,
    )
    .await
    .inspect_err(|e| error!("Error starting Sequencer: {e}")) else {
        return Ok(());
    };

    if needed_proof_types.contains(&ProverType::Aligned) && !cfg.aligned.aligned_mode {
        error!(
            "Aligned mode is required. Please set the `--aligned` flag or use the `ALIGNED_MODE` environment variable to true."
        );
        return Ok(());
    }
    if needed_proof_types.contains(&ProverType::TDX)
        && cfg.proof_coordinator.tdx_private_key.is_none()
    {
        error!(
            "A private key for TDX is required. Please set the flag `--proof-coordinator.tdx-private-key <KEY>` or use the `ETHREX_PROOF_COORDINATOR_TDX_PRIVATE_KEY` environment variable to set the private key"
        );
        return Ok(());
    }

    let l1_watcher = L1Watcher::spawn(
        store.clone(),
        blockchain.clone(),
        cfg.clone(),
        shared_state.clone(),
    )
    .await
    .inspect_err(|err| {
        error!("Error starting Watcher: {err}");
    });
    let l1_committer = L1Committer::spawn(
        store.clone(),
        blockchain.clone(),
        rollup_store.clone(),
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

    let l1_proof_sender = L1ProofSender::spawn(
        cfg.clone(),
        shared_state.clone(),
        rollup_store.clone(),
        needed_proof_types.clone(),
    )
    .await
    .inspect_err(|err| {
        error!("Error starting L1 Proof Sender: {err}");
    });
    let block_producer = BlockProducer::spawn(
        store.clone(),
        rollup_store.clone(),
        blockchain.clone(),
        cfg.clone(),
        shared_state.clone(),
    )
    .await
    .inspect_err(|err| {
        error!("Error starting Block Producer: {err}");
    });

    #[cfg(feature = "metrics")]
    let metrics_gatherer = MetricsGatherer::spawn(&cfg, rollup_store.clone(), l2_url)
        .await
        .inspect_err(|err| {
            error!("Error starting Block Producer: {err}");
        });
    let mut verifier_handle = None;

    if needed_proof_types.contains(&ProverType::Aligned) {
        verifier_handle = Some(tokio::spawn(l1_proof_verifier::start_l1_proof_verifier(
            cfg.clone(),
            rollup_store.clone(),
        )));
    }
    let (state_updater, block_fetcher) = if cfg.based.enabled {
        let state_updater = StateUpdater::spawn(
            cfg.clone(),
            shared_state.clone(),
            blockchain.clone(),
            store.clone(),
            rollup_store.clone(),
        )
        .await
        .inspect_err(|err| {
            error!("Error starting State Updater: {err}");
        });

        let block_fetcher = BlockFetcher::spawn(
            &cfg,
            store.clone(),
            rollup_store.clone(),
            blockchain,
            shared_state.clone(),
        )
        .await
        .inspect_err(|err| {
            error!("Error starting Block Fetcher: {err}");
        });
        (state_updater.ok(), block_fetcher.ok())
    } else {
        (None, None)
    };

    if cfg.monitor.enabled {
        EthrexMonitor::spawn(
            shared_state.clone(),
            store.clone(),
            rollup_store.clone(),
            &cfg,
            cancellation_token.clone(),
        )
        .await?;
    }

    let admin_server = start_api(
        format!(
            "{}:{}",
            cfg.admin_server.listen_ip, cfg.admin_server.listen_port
        ),
        l1_committer.ok(),
        l1_watcher.ok(),
        l1_proof_sender.ok(),
        block_producer.ok(),
        #[cfg(feature = "metrics")]
        metrics_gatherer.ok(),
        state_updater,
        block_fetcher,
    )
    .await
    .inspect_err(|err| {
        error!("Error starting admin server: {err}");
    })
    .ok();

    match (verifier_handle, admin_server) {
        (Some(handle), Some(admin_server)) => {
            let (server_res, verifier_res) = tokio::join!(admin_server.into_future(), handle);
            if let Err(e) = server_res {
                error!("Admin server task error: {e}");
            }
            handle_verifier_result(verifier_res).await;
        }
        (Some(handle), None) => {
            handle_verifier_result(tokio::join!(handle).0).await;
        }
        (None, Some(admin_server)) => {
            if let Err(e) = admin_server.into_future().await {
                error!("Admin server task error: {e}");
            }
        }
        (None, None) => {}
    }

    Ok(())
}

async fn handle_verifier_result(res: Result<Result<(), SequencerError>, tokio::task::JoinError>) {
    match res {
        Ok(Ok(_)) => {}
        Ok(Err(err)) => error!("verifier error: {err}"),
        Err(err) => error!("verifier task join error: {err}"),
    }
}
