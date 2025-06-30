use std::{sync::Arc, time::Duration};

use ethrex_blockchain::{
    Blockchain,
    fork_choice::apply_fork_choice,
    sequencer_state::{SequencerState, SequencerStatus},
};
use ethrex_common::{Address, U256, types::Block};
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_p2p::sync_manager::SyncManager;
use ethrex_rpc::{EthClient, clients::Overrides};
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use keccak_hash::keccak;
use spawned_concurrency::{CallResponse, CastResponse, GenServer, GenServerError, send_after};
use tracing::{debug, error, info, warn};

use crate::{SequencerConfig, sequencer::utils::node_is_up_to_date, utils::parse::hash_to_address};

#[derive(Debug, thiserror::Error)]
pub enum StateUpdaterError {
    #[error("State Updater failed due to an EthClient error: {0}")]
    EthClientError(#[from] ethrex_rpc::clients::EthClientError),
    #[error("State Updater failed when trying to encode the calldata: {0}")]
    CalldataEncodeError(#[from] ethrex_rpc::clients::eth::errors::CalldataEncodeError),
    #[error("State Updater failed when trying to parse the calldata: {0}")]
    CalldataParsingError(String),
    #[error("State Updater failed due to a Store error: {0}")]
    StoreError(#[from] ethrex_storage::error::StoreError),
    #[error("Failed to apply fork choice for fetched block: {0}")]
    InvalidForkChoice(#[from] ethrex_blockchain::error::InvalidForkChoice),
    #[error("Internal Error: {0}")]
    InternalError(String),
    #[error("Spawned GenServer Error")]
    GenServerError(GenServerError),
}

#[derive(Clone)]
pub struct StateUpdaterState {
    on_chain_proposer_address: Address,
    sequencer_registry_address: Address,
    sequencer_address: Address,
    eth_client: Arc<EthClient>,
    store: Store,
    rollup_store: StoreRollup,
    check_interval_ms: u64,
    sequencer_state: SequencerState,
    blockchain: Arc<Blockchain>,
    sync_manager: SyncManager,
    latest_block_fetched: U256,
}

impl StateUpdaterState {
    pub fn new(
        sequencer_cfg: SequencerConfig,
        sequencer_state: SequencerState,
        blockchain: Arc<Blockchain>,
        store: Store,
        rollup_store: StoreRollup,
        sync_manager: SyncManager,
    ) -> Result<Self, StateUpdaterError> {
        Ok(Self {
            on_chain_proposer_address: sequencer_cfg.l1_committer.on_chain_proposer_address,
            sequencer_registry_address: sequencer_cfg.based.state_updater.sequencer_registry,
            sequencer_address: sequencer_cfg.l1_committer.l1_address,
            eth_client: Arc::new(EthClient::new_with_multiple_urls(
                sequencer_cfg.eth.rpc_url.clone(),
            )?),
            store,
            rollup_store,
            check_interval_ms: sequencer_cfg.based.state_updater.check_interval_ms,
            sequencer_state,
            blockchain,
            sync_manager,
            latest_block_fetched: U256::zero(),
        })
    }
}

#[derive(Clone)]
pub enum InMessage {
    UpdateState,
}

#[derive(Clone, PartialEq)]
pub enum OutMessage {
    Done,
}

pub struct StateUpdater;

impl StateUpdater {
    pub async fn spawn(
        sequencer_cfg: SequencerConfig,
        sequencer_state: SequencerState,
        blockchain: Arc<Blockchain>,
        store: Store,
        rollup_store: StoreRollup,
        sync_manager: SyncManager,
    ) -> Result<(), StateUpdaterError> {
        let state = StateUpdaterState::new(
            sequencer_cfg,
            sequencer_state,
            blockchain,
            store,
            rollup_store,
            sync_manager,
        )?;
        let mut state_updater = StateUpdater::start(state);
        state_updater
            .cast(InMessage::UpdateState)
            .await
            .map_err(StateUpdaterError::GenServerError)
    }
}

impl GenServer for StateUpdater {
    type InMsg = InMessage;
    type OutMsg = OutMessage;
    type State = StateUpdaterState;
    type Error = StateUpdaterError;

    fn new() -> Self {
        Self {}
    }

    async fn handle_call(
        &mut self,
        _message: Self::InMsg,
        _tx: &spawned_rt::mpsc::Sender<spawned_concurrency::GenServerInMsg<Self>>,
        _state: &mut Self::State,
    ) -> spawned_concurrency::CallResponse<Self::OutMsg> {
        CallResponse::Reply(OutMessage::Done)
    }

    async fn handle_cast(
        &mut self,
        _message: Self::InMsg,
        tx: &spawned_rt::mpsc::Sender<spawned_concurrency::GenServerInMsg<Self>>,
        state: &mut Self::State,
    ) -> spawned_concurrency::CastResponse {
        let _ = update_state(state)
            .await
            .inspect_err(|err| error!("State Updater Error: {err}"));
        send_after(
            Duration::from_millis(state.check_interval_ms),
            tx.clone(),
            Self::InMsg::UpdateState,
        );
        CastResponse::NoReply
    }
}

pub async fn update_state(state: &mut StateUpdaterState) -> Result<(), StateUpdaterError> {
    let lead_sequencer = hash_to_address(
        state
            .eth_client
            .call(
                state.sequencer_registry_address,
                encode_calldata("leaderSequencer()", &[])?.into(),
                Overrides::default(),
            )
            .await?
            .parse()
            .map_err(|_| {
                StateUpdaterError::CalldataParsingError(
                    "Failed to parse leaderSequencer() return data".to_string(),
                )
            })?,
    );

    let node_is_up_to_date = node_is_up_to_date::<StateUpdaterError>(
        &state.eth_client,
        state.on_chain_proposer_address,
        &state.rollup_store,
    )
    .await?;

    let current_state = state.sequencer_state.status().await;

    let new_status = determine_new_status(
        &current_state,
        node_is_up_to_date,
        lead_sequencer == state.sequencer_address,
    );
    if let SequencerStatus::Syncing = new_status {
        if !state.blockchain.is_synced() {
            let latest_l1_block = state.eth_client.get_block_number().await?;
            let latest_batch_committed = state
                .eth_client
                .get_last_committed_batch(state.on_chain_proposer_address)
                .await?;
            // dbg!(latest_batch_committed);
            // dbg!(
            //     state
            //         .rollup_store
            //         .contains_batch(&latest_batch_committed)
            //         .await?
            // );
            let logs = state
                .eth_client
                .get_logs(
                    state.latest_block_fetched,
                    latest_l1_block,
                    state.on_chain_proposer_address,
                    keccak(b"BatchCommitted(uint256,bytes32,bytes32)"),
                )
                .await?;
            if let Some(last_committed_batch_log) = logs.last() {
                let newest_fcu_head =
                    last_committed_batch_log.log.topics.get(3).ok_or_else(|| {
                        StateUpdaterError::InternalError(
                            "BatchCommitted log does not contain enough topics".to_string(),
                        )
                    })?;
                state
                    .sync_manager
                    .sync_to_head(*newest_fcu_head, latest_batch_committed);
                if !state.sync_manager.is_active() {
                    // dbg!("Sync manager is not active, updating latest block fetched.");
                    state.latest_block_fetched = latest_l1_block;
                }
                // state.blockchain.set_synced();
            } else {
                // warn!("No new BatchCommitted logs found, continuing to sync.");
            }
        }
    }

    if current_state != new_status {
        info!("State transition: {:?} -> {:?}", current_state, new_status);

        if current_state == SequencerStatus::Sequencing {
            info!("Stopping sequencing.");
            revert_uncommitted_state(state).await?;
        }

        if new_status == SequencerStatus::Sequencing {
            info!("Starting sequencing as lead sequencer.");
            revert_uncommitted_state(state).await?;
        }

        match new_status {
            // This case is handled above, it is redundant here.
            SequencerStatus::Sequencing => {
                info!("Node is now the lead sequencer.");
            }
            SequencerStatus::Following => {
                state.blockchain.set_synced();
                info!("Node is up to date and following the lead sequencer.");
            }
            SequencerStatus::Syncing => {
                state.blockchain.set_not_synced();
                info!("Node is synchronizing to catch up with the latest state.");
            }
        }
    }

    // Update the state
    state.sequencer_state.new_status(new_status).await;

    Ok(())
}

fn determine_new_status(
    current_state: &SequencerStatus,
    node_is_up_to_date: bool,
    is_lead_sequencer: bool,
) -> SequencerStatus {
    match (node_is_up_to_date, is_lead_sequencer) {
        // A node can be the lead sequencer only if it is up to date.
        (true, true) => {
            if *current_state == SequencerStatus::Syncing {
                SequencerStatus::Following
            } else {
                SequencerStatus::Sequencing
            }
        }
        // If the node is up to date but not the lead sequencer, it follows the lead sequencer.
        (true, false) => {
            info!("Node is up to date and following the lead sequencer.");
            SequencerStatus::Following
        }
        // If the node is not up to date, it should sync.
        (false, _) => {
            if is_lead_sequencer && *current_state == SequencerStatus::Syncing {
                warn!("Node is not up to date but is the lead sequencer, continue syncing.");
            }
            info!("Node is not up to date, syncing...");
            SequencerStatus::Syncing
        }
    }
}

/// Reverts state to the last committed batch if known.
async fn revert_uncommitted_state(state: &mut StateUpdaterState) -> Result<(), StateUpdaterError> {
    let last_l2_committed_batch = state
        .eth_client
        .get_last_committed_batch(state.on_chain_proposer_address)
        .await?;

    debug!("Last committed batch: {last_l2_committed_batch}");

    let Some(last_l2_committed_batch_blocks) = state
        .rollup_store
        .get_block_numbers_by_batch(last_l2_committed_batch)
        .await?
    else {
        // Node is not up to date. There is no uncommitted state to revert.
        info!("No uncommitted state to revert. Node is not up to date.");
        return Ok(());
    };

    debug!(
        "Last committed batch blocks: {:?}",
        last_l2_committed_batch_blocks
    );

    let Some(last_l2_committed_block_number) = last_l2_committed_batch_blocks.last() else {
        return Err(StateUpdaterError::InternalError(format!(
            "No blocks found for the last committed batch {last_l2_committed_batch}"
        )));
    };

    debug!("Last committed batch block number: {last_l2_committed_block_number}");

    let last_l2_committed_block_body = state
        .store
        .get_block_body(*last_l2_committed_block_number)
        .await?
        .ok_or(StateUpdaterError::InternalError(
            "No block body found for the last committed batch block number".to_string(),
        ))?;

    let last_l2_committed_block_header = state
        .store
        .get_block_header(*last_l2_committed_block_number)?
        .ok_or(StateUpdaterError::InternalError(
            "No block header found for the last committed batch block number".to_string(),
        ))?;

    let last_l2_committed_batch_block =
        Block::new(last_l2_committed_block_header, last_l2_committed_block_body);

    let last_l2_committed_batch_block_hash = last_l2_committed_batch_block.hash();

    info!(
        "Reverting uncommitted state to the last committed batch block {last_l2_committed_block_number} with hash {last_l2_committed_batch_block_hash:#x}"
    );
    state
        .store
        .update_latest_block_number(*last_l2_committed_block_number)
        .await?;
    state
        .store
        .unset_canonical_block(*last_l2_committed_block_number + 1)
        .await?;
    let _ = apply_fork_choice(
        &state.store,
        last_l2_committed_batch_block_hash,
        last_l2_committed_batch_block_hash,
        last_l2_committed_batch_block_hash,
    )
    .await
    .map_err(StateUpdaterError::InvalidForkChoice)?;
    Ok(())
}
