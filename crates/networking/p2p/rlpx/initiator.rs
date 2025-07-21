use std::time::Duration;

use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, send_after},
};

use tracing::{debug, info};

use crate::{metrics::METRICS, network::P2PContext};

use crate::rlpx::connection::server::RLPxConnection;

#[derive(Debug, thiserror::Error)]
pub enum RLPxInitiatorError {
    // #[error(transparent)]
    // IoError(#[from] std::io::Error),
    // #[error("Failed to send message")]
    // MessageSendFailure(std::io::Error),
    // #[error("Only partial message was sent")]
    // PartialMessageSent,
}

#[derive(Debug, Clone)]
pub struct RLPxInitiatorState {
    context: P2PContext,

    /// The initial interval between peer lookups, until the number of peers
    /// reaches [target_peers](RLPxInitiatorState::target_peers).
    initial_lookup_interval: Duration,
    lookup_interval: Duration,

    /// The target number of RLPx connections to reach.
    target_peers: u64,
    /// The rate at which to try new connections.
    new_connections_per_lookup: u64,
}

impl RLPxInitiatorState {
    pub fn new(context: P2PContext) -> Self {
        Self {
            context,
            initial_lookup_interval: Duration::from_secs(3),
            lookup_interval: Duration::from_secs(5 * 60),
            target_peers: 50,
            new_connections_per_lookup: 5000,
        }
    }
}

#[derive(Debug, Clone)]
pub enum InMessage {
    LookForPeers,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

#[derive(Debug, Default)]
pub struct RLPxInitiator;

impl RLPxInitiator {
    pub async fn spawn(context: P2PContext) -> Result<(), RLPxInitiatorError> {
        info!("Starting RLPx Initiator");

        let state = RLPxInitiatorState::new(context);

        let mut server = RLPxInitiator::start(state.clone());

        let _ = server.cast(InMessage::LookForPeers).await;

        Ok(())
    }
}

impl GenServer for RLPxInitiator {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type State = RLPxInitiatorState;
    type Error = RLPxInitiatorError;

    fn new() -> Self {
        Self {}
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
        state: Self::State,
    ) -> CastResponse<Self> {
        match message {
            Self::CastMsg::LookForPeers => {
                debug!(received = "Look for peers");

                look_for_peers(&state).await;

                send_after(
                    get_lookup_interval(&state).await,
                    handle.clone(),
                    Self::CastMsg::LookForPeers,
                );

                CastResponse::NoReply(state)
            }
        }
    }
}

async fn look_for_peers(state: &RLPxInitiatorState) {
    info!("Looking for peers");

    let mut already_tried_peers = state.context.table.already_tried_peers.lock().await;

    let mut tried_connections = 0;

    for contact in state.context.table.table.lock().await.values() {
        let node_id = contact.node.node_id();
        if !already_tried_peers.contains(&node_id) && contact.knows_us {
            already_tried_peers.insert(node_id);

            RLPxConnection::spawn_as_initiator(state.context.clone(), &contact.node).await;

            METRICS.record_new_rlpx_conn_attempt().await;
            tried_connections += 1;
            if tried_connections >= state.new_connections_per_lookup {
                break;
            }
        }
    }

    if tried_connections < state.new_connections_per_lookup {
        info!("Resetting list of tried peers.");
        already_tried_peers.clear();
    }
}

async fn get_lookup_interval(state: &RLPxInitiatorState) -> Duration {
    let num_peers = state.context.table.peers.lock().await.len() as u64;

    if num_peers < state.target_peers {
        state.initial_lookup_interval
    } else {
        info!("Reached target number of peers. Using longer lookup interval.");
        state.lookup_interval
    }
}
