use std::{sync::Arc, time::Duration};

use ethrex_common::Address;
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_rpc::{clients::Overrides, EthClient};
use tokio::{sync::Mutex, time::sleep};
use tracing::{error, info};

use crate::{utils::parse::hash_to_address, SequencerConfig};

use super::{
    errors::{SequencerError, StateUpdaterError},
    SequencerState,
};

pub struct StateUpdater {
    sequencer_registry: Address,
    sequencer_address: Address,
    eth_client: Arc<EthClient>,
    check_interval_ms: u64,
}

pub async fn start_state_updater(
    sequencer_cfg: SequencerConfig,
    sequencer_state: Arc<Mutex<SequencerState>>,
) -> Result<(), SequencerError> {
    let state_updater = StateUpdater::new(sequencer_cfg)?;
    state_updater.run(sequencer_state).await;
    Ok(())
}

impl StateUpdater {
    pub fn new(sequencer_cfg: SequencerConfig) -> Result<Self, StateUpdaterError> {
        Ok(Self {
            sequencer_registry: sequencer_cfg.state_updater.sequencer_registry,
            sequencer_address: sequencer_cfg.l1_committer.l1_address,
            eth_client: Arc::new(EthClient::new_with_multiple_urls(
                sequencer_cfg.eth.rpc_url.clone(),
            )?),
            check_interval_ms: sequencer_cfg.state_updater.check_interval_ms,
        })
    }

    pub async fn run(&self, sequencer_state: Arc<Mutex<SequencerState>>) {
        loop {
            if let Err(err) = self.main_logic(sequencer_state.clone()).await {
                error!("State Updater Error: {}", err);
            }

            sleep(Duration::from_millis(self.check_interval_ms)).await;
        }
    }

    pub async fn main_logic(
        &self,
        sequencer_state: Arc<Mutex<SequencerState>>,
    ) -> Result<(), StateUpdaterError> {
        let calldata = encode_calldata("leaderSequencer()", &[])?;

        let leader_sequencer = hash_to_address(
            self.eth_client
                .call(
                    self.sequencer_registry,
                    calldata.into(),
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

        let mut current_state = sequencer_state.lock().await;

        let new_state = if leader_sequencer == self.sequencer_address {
            SequencerState::Sequencing
        } else {
            SequencerState::Following
        };

        match (current_state.clone(), new_state.clone()) {
            (SequencerState::Sequencing, SequencerState::Sequencing)
            | (SequencerState::Following, SequencerState::Following) => {}
            (SequencerState::Sequencing, SequencerState::Following) => {
                info!("Now the follower sequencer. Stopping Sequencing.");
            }
            (SequencerState::Following, SequencerState::Sequencing) => {
                info!("Now the leader sequencer. Starting Sequencing.");
            }
        };

        *current_state = new_state;

        Ok(())
    }
}
