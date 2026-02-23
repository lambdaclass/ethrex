use std::path::PathBuf;
use std::sync::Arc;

use crate::sequencer::admin_server::start_api;
use crate::sequencer::errors::SequencerError;
use crate::sequencer::state_updater::StateUpdater;
use crate::{BlockFetcher, SequencerConfig};
use block_producer::BlockProducer;
use ethrex_blockchain::Blockchain;
use ethrex_common::types::Genesis;
use ethrex_l2_common::prover::ProverType;
use ethrex_monitor::{EthrexMonitor, MonitorConfig as ExternalMonitorConfig};
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use l1_committer::L1Committer;
use l1_proof_sender::L1ProofSender;
use l1_watcher::L1Watcher;
#[cfg(feature = "metrics")]
use metrics::MetricsGatherer;
use proof_coordinator::ProofCoordinator;
use reqwest::Url;
use spawned_concurrency::tasks::GenServerHandle;
use std::pin::Pin;
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
pub use ethrex_l2_common::sequencer_state::{SequencerState, SequencerStatus};
pub mod state_updater;

pub mod configs;
pub mod errors;
pub mod setup;
pub mod utils;

#[expect(clippy::too_many_arguments)]
pub async fn start_l2(
    store: Store,
    rollup_store: StoreRollup,
    blockchain: Arc<Blockchain>,
    cfg: SequencerConfig,
    cancellation_token: CancellationToken,
    l2_url: Url,
    genesis: Genesis,
    checkpoints_dir: PathBuf,
) -> Result<
    (
        Option<GenServerHandle<L1Committer>>,
        Option<GenServerHandle<BlockProducer>>,
        Pin<Box<dyn Future<Output = Result<(), errors::SequencerError>> + Send>>,
    ),
    errors::SequencerError,
> {
    let initial_status = if cfg.based.enabled {
        SequencerStatus::default()
    } else if cfg.state_updater.start_at > 0 {
        SequencerStatus::Syncing
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
        return Ok((
            None,
            None,
            Box::pin(async { Ok::<(), errors::SequencerError>(()) }),
        ));
    };

    if needed_proof_types.contains(&ProverType::TDX)
        && cfg.proof_coordinator.tdx_private_key.is_none()
    {
        error!(
            "A private key for TDX is required. Please set the flag `--proof-coordinator.tdx-private-key <KEY>` or use the `ETHREX_PROOF_COORDINATOR_TDX_PRIVATE_KEY` environment variable to set the private key"
        );
        return Ok((
            None,
            None,
            Box::pin(async { Ok::<(), errors::SequencerError>(()) }),
        ));
    }

    let l1_watcher = L1Watcher::spawn(
        store.clone(),
        blockchain.clone(),
        cfg.clone(),
        shared_state.clone(),
        l2_url.clone(),
    )
    .inspect_err(|err| {
        error!("Error starting Watcher: {err}");
    });
    let l1_committer = L1Committer::spawn(
        store.clone(),
        blockchain.clone(),
        rollup_store.clone(),
        cfg.clone(),
        shared_state.clone(),
        genesis,
        checkpoints_dir.clone(),
    )
    .await
    .inspect_err(|err| {
        error!("Error starting Committer: {err}");
    });
    let _ = ProofCoordinator::spawn(
        rollup_store.clone(),
        cfg.clone(),
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
        checkpoints_dir,
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
        cfg.l1_watcher.router_address,
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

    if cfg.aligned.aligned_mode {
        verifier_handle = Some(tokio::spawn(l1_proof_verifier::start_l1_proof_verifier(
            cfg.clone(),
            rollup_store.clone(),
            needed_proof_types.clone(),
        )));
    }
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
    let block_fetcher = if cfg.based.enabled {
        BlockFetcher::spawn(
            &cfg,
            store.clone(),
            rollup_store.clone(),
            blockchain,
            shared_state.clone(),
        )
        .await
        .inspect_err(|err| {
            error!("Error starting Block Fetcher: {err}");
        })
        .ok()
    } else {
        None
    };

    if cfg.monitor.enabled {
        let monitor_cfg = monitor_config_from(&cfg);
        EthrexMonitor::spawn(
            shared_state.clone(),
            store.clone(),
            rollup_store.clone(),
            &monitor_cfg,
            cancellation_token.clone(),
        )
        .await?;
    }

    let l1_committer_handle = l1_committer.ok();
    let block_producer_handle = block_producer.ok();
    let admin_server = start_api(
        format!(
            "{}:{}",
            cfg.admin_server.listen_ip, cfg.admin_server.listen_port
        ),
        l1_committer_handle.clone(),
        l1_watcher.ok(),
        l1_proof_sender.ok(),
        block_producer_handle.clone(),
        state_updater.ok(),
        block_fetcher,
        #[cfg(feature = "metrics")]
        metrics_gatherer.ok(),
    )
    .await
    .inspect_err(|err| {
        error!("Error starting admin server: {err}");
    })
    .ok();

    let driver = Box::pin(async move {
        match (verifier_handle, admin_server) {
            (Some(handle), Some(admin_server)) => {
                let (server_res, verifier_res) = tokio::join!(admin_server.into_future(), handle);
                if let Err(e) = server_res {
                    error!("Admin server task error: {e}");
                }
                handle_verifier_result(verifier_res);
            }
            (Some(handle), None) => handle_verifier_result(tokio::join!(handle).0),
            (None, Some(admin_server)) => {
                if let Err(e) = admin_server.into_future().await {
                    error!("Admin server task error: {e}");
                }
            }
            (None, None) => {}
        }

        Ok(())
    });
    Ok((l1_committer_handle, block_producer_handle, driver))
}

fn monitor_config_from(cfg: &SequencerConfig) -> ExternalMonitorConfig {
    let sequencer_registry_address =
        if cfg.state_updater.sequencer_registry == ethrex_common::Address::default() {
            None
        } else {
            Some(cfg.state_updater.sequencer_registry)
        };
    ExternalMonitorConfig {
        enabled: cfg.monitor.enabled,
        tick_rate: cfg.monitor.tick_rate,
        batch_widget_height: cfg.monitor.batch_widget_height,
        on_chain_proposer_address: cfg.l1_committer.on_chain_proposer_address,
        bridge_address: cfg.l1_watcher.bridge_address,
        sequencer_registry_address,
        rpc_urls: cfg.eth.rpc_url.clone(),
        is_based: cfg.based.enabled,
        osaka_activation_time: cfg.eth.osaka_activation_time,
    }
}

fn handle_verifier_result(res: Result<Result<(), SequencerError>, tokio::task::JoinError>) {
    match res {
        Ok(Ok(_)) => {}
        Ok(Err(err)) => error!("verifier error: {err}"),
        Err(err) => error!("verifier task join error: {err}"),
    }
}
