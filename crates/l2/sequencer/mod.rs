use std::sync::Arc;

use crate::based::sequencer_state::SequencerState;
use crate::based::sequencer_state::SequencerStatus;
use crate::monitor::EthrexMonitor;
use crate::{BlockFetcher, SequencerConfig, StateUpdater};
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

    if let (Some(block_gas_limit), Some(batch_gas_limit)) = (
        cfg.block_producer.max_gas_limit,
        cfg.l1_committer.batch_gas_limit,
    ) {
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
    let _ = MetricsGatherer::spawn(&cfg, rollup_store.clone(), l2_url)
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
    if cfg.based.enabled {
        let _ = StateUpdater::spawn(
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

        let _ = BlockFetcher::spawn(
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
    }

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

    let Some(handle) = verifier_handle else {
        return Ok(());
    };

    match handle.await {
        Ok(Ok(_)) => {}
        Ok(Err(err)) => {
            error!("Error running verifier: {err}");
        }
        Err(err) => {
            error!("Task error: {err}");
        }
    };

    Ok(())
}
