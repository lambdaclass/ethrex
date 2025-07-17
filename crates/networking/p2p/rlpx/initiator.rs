use std::{str::FromStr, time::Duration};

use ethrex_common::H256;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, send_after},
};

use tracing::{debug, info};

use crate::{metrics::METRICS, network::P2PContext, types::Node};

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
    _geth_peers: Vec<H256>,
    context: P2PContext,
    lookup_period: Duration,
}

impl RLPxInitiatorState {
    pub fn new(context: P2PContext) -> Self {
        let _geth_peers =
            serde_json::from_str::<Vec<String>>(include_str!("../../../../geth_peers.json"))
                .expect("Failed to parse geth_peers.json")
                .iter()
                .map(|e| {
                    Node::from_str(e)
                        .expect("Failed to parse bootnode enode")
                        .node_id()
                })
                .collect::<Vec<_>>();
        Self {
            _geth_peers,
            context,
            lookup_period: Duration::from_secs(3),
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
                    state.lookup_period,
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

    let mut already_known_peers_table = state.context.table.already_tried_peers.lock().await;

    for contact in state.context.table.table.lock().await.values() {
        let node_id = contact.node.node_id();
        if !already_known_peers_table.contains(&node_id) && contact.knows_us {
            already_known_peers_table.insert(node_id);

            RLPxConnection::spawn_as_initiator(state.context.clone(), &contact.node).await;

            METRICS.record_new_rlpx_conn_attempt().await;
            if state._geth_peers.contains(&node_id) {
                METRICS
                    .new_connection_attempt_to_mainnet_peer(node_id)
                    .await;
            }
        }
    }
}
