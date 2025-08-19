use std::{sync::Arc, time::Duration};

use ethrex_blockchain::Blockchain;
use ethrex_common::{Address, types::Block};
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_rpc::{EthClient, clients::Overrides};
use ethrex_storage::Store;
use ethrex_storage_rollup::{RollupStoreError, StoreRollup};
use spawned_concurrency::{
    error::GenServerError,
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, send_after},
};
use tracing::{debug, error, info, warn};

use crate::{
    SequencerConfig,
    based::sequencer_state::{SequencerState, SequencerStatus},
    sequencer::utils::node_is_up_to_date,
    utils::parse::hash_to_address,
};

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
    #[error("State Updater failed due to a RollupStore error: {0}")]
    RollupStoreError(#[from] RollupStoreError),
    #[error("Failed to apply fork choice for fetched block: {0}")]
    InvalidForkChoice(#[from] ethrex_blockchain::error::InvalidForkChoice),
    #[error("Internal Error: {0}")]
    InternalError(String),
    // TODO: Avoid propagating GenServerErrors outside GenServer modules
    // See https://github.com/lambdaclass/ethrex/issues/3376
    #[error("Spawned GenServer Error")]
    GenServerError(GenServerError),
}

#[derive(Clone)]
pub enum InMessage {
    UpdateState,
}

#[derive(Clone, PartialEq)]
pub enum OutMessage {
    Done,
}

pub struct StateUpdater {
    on_chain_proposer_address: Address,
    sequencer_registry_address: Address,
    sequencer_address: Address,
    eth_client: Arc<EthClient>,
    store: Store,
    rollup_store: StoreRollup,
    check_interval_ms: u64,
    sequencer_state: SequencerState,
    blockchain: Arc<Blockchain>,
}

impl StateUpdater {
    pub fn new(
        sequencer_cfg: SequencerConfig,
        sequencer_state: SequencerState,
        blockchain: Arc<Blockchain>,
        store: Store,
        rollup_store: StoreRollup,
    ) -> Result<Self, StateUpdaterError> {
        Ok(Self {
            on_chain_proposer_address: sequencer_cfg.l1_committer.on_chain_proposer_address,
            sequencer_registry_address: sequencer_cfg.based.state_updater.sequencer_registry,
            sequencer_address: sequencer_cfg.l1_committer.signer.address(),
            eth_client: Arc::new(EthClient::new_with_multiple_urls(
                sequencer_cfg.eth.rpc_url.clone(),
            )?),
            store,
            rollup_store,
            check_interval_ms: sequencer_cfg.based.state_updater.check_interval_ms,
            sequencer_state,
            blockchain,
        })
    }

    pub async fn spawn(
        sequencer_cfg: SequencerConfig,
        sequencer_state: SequencerState,
        blockchain: Arc<Blockchain>,
        store: Store,
        rollup_store: StoreRollup,
    ) -> Result<(), StateUpdaterError> {
        let mut state_updater = Self::new(
            sequencer_cfg,
            sequencer_state,
            blockchain,
            store,
            rollup_store,
        )?
        .start();
        state_updater
            .cast(InMessage::UpdateState)
            .await
            .map_err(StateUpdaterError::GenServerError)
    }

    pub async fn update_state(&mut self) -> Result<(), StateUpdaterError> {
        let lead_sequencer = hash_to_address(
            self.eth_client
                .call(
                    self.sequencer_registry_address,
                    encode_calldata("leaderSequencer()", &[])?.into(),
                    Overrides::default(),
                )
                .await?
                .parse()
                .map_err(|err| {
                    StateUpdaterError::CalldataParsingError(format!(
                        "Failed to parse leaderSequencer() return data: {err}"
                    ))
                })?,
        );

        let node_is_up_to_date = node_is_up_to_date::<StateUpdaterError>(
            &self.eth_client,
            self.on_chain_proposer_address,
            &self.rollup_store,
        )
        .await?;

        let current_state = self.sequencer_state.status().await;

        let new_status = determine_new_status(
            current_state,
            node_is_up_to_date,
            lead_sequencer == self.sequencer_address,
        );

        if current_state != new_status {
            info!("State transition: {:?} -> {:?}", current_state, new_status);

            if current_state == SequencerStatus::Sequencing {
                info!("Stopping sequencing.");
                self.revert_uncommitted_state().await?;
            }

            if new_status == SequencerStatus::Sequencing {
                info!("Starting sequencing as lead sequencer.");
                self.revert_uncommitted_state().await?;
            }

            match new_status {
                // This case is handled above, it is redundant here.
                SequencerStatus::Sequencing => {
                    info!("Node is now the lead sequencer.");
                }
                SequencerStatus::Following => {
                    self.blockchain.set_synced();
                    info!("Node is up to date and following the lead sequencer.");
                }
                SequencerStatus::Syncing => {
                    self.blockchain.set_not_synced();
                    info!("Node is synchronizing to catch up with the latest state.");
                }
            }
        }

        // Update the state
        self.sequencer_state.new_status(new_status).await;

        Ok(())
    }

    /// Reverts state to the last committed batch if known.
    async fn revert_uncommitted_state(&mut self) -> Result<(), StateUpdaterError> {
        let last_l2_committed_batch = self
            .eth_client
            .get_last_committed_batch(self.on_chain_proposer_address)
            .await?;

        debug!("Last committed batch: {last_l2_committed_batch}");

        let Some(last_l2_committed_batch_blocks) = self
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

        let last_l2_committed_block_body = self
            .store
            .get_block_body(*last_l2_committed_block_number)
            .await?
            .ok_or(StateUpdaterError::InternalError(
                "No block body found for the last committed batch block number".to_string(),
            ))?;

        let last_l2_committed_block_header = self
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
        self.store
            .forkchoice_update(
                None,
                *last_l2_committed_block_number,
                last_l2_committed_batch_block_hash,
                None,
                None,
            )
            .await?;

        Ok(())
    }
}

impl GenServer for StateUpdater {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = StateUpdaterError;

    async fn handle_cast(
        &mut self,
        _message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        let _ = self
            .update_state()
            .await
            .inspect_err(|err| error!("State Updater Error: {err}"));
        send_after(
            Duration::from_millis(self.check_interval_ms),
            handle.clone(),
            Self::CastMsg::UpdateState,
        );
        CastResponse::NoReply
    }
}

fn determine_new_status(
    current_state: SequencerStatus,
    node_is_up_to_date: bool,
    is_lead_sequencer: bool,
) -> SequencerStatus {
    match (node_is_up_to_date, is_lead_sequencer) {
        // A node can be the lead sequencer only if it is up to date.
        (true, true) => {
            if current_state == SequencerStatus::Syncing {
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
            if is_lead_sequencer && current_state == SequencerStatus::Syncing {
                warn!("Node is not up to date but is the lead sequencer, continue syncing.");
            }
            SequencerStatus::Syncing
        }
    }
}
