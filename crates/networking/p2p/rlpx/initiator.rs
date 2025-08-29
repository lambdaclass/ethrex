use std::{sync::LazyLock, time::Duration};

use spawned_concurrency::{
    messages::Unused,
    tasks::GenServerHandle,
    tasks::{CastResponse, GenServer, send_after},
};

use tokio::{sync::OnceCell, time::Instant};
use tracing::{debug, error, info};

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
pub struct RLPxInitiator {
    context: P2PContext,

    /// The initial interval between peer lookups, until the number of peers
    /// reaches [target_peers](RLPxInitiatorState::target_peers).
    initial_lookup_interval: Duration,
    lookup_interval: Duration,

    /// Interval for logging the amount for peers and clearing the table of aleeady connected peers.
    last_log_time: Instant,

    /// The target number of RLPx connections to reach.
    target_peers: u64,
}

impl RLPxInitiator {
    pub fn new(context: P2PContext) -> Self {
        Self {
            context,
            initial_lookup_interval: Duration::from_secs(3),
            lookup_interval: Duration::from_secs(5 * 60),
            target_peers: 500,
            last_log_time: Instant::now(),
        }
    }

    pub async fn spawn(context: P2PContext) -> Result<(), RLPxInitiatorError> {
        info!("Starting RLPx Initiator");

        let state = RLPxInitiator::new(context);

        let server = RLPxInitiator::start(state.clone());
        if let Err(err) = INITIATOR.set(server) {
            error!("We tried to start multiple RLPxInitiators: {err}");
        };

        for _ in 0..state.target_peers {
            let _ = INITIATOR
                .get()
                .expect("We should get the initiator we just set up")
                .clone()
                .cast(InMessage::LookForPeer)
                .await;
        }

        Ok(())
    }

    /// Looks for a single peer. If it finds one to attempt a connection, returns true
    /// Else returns false
    async fn look_for_peer(&mut self) -> bool {
        let mut already_tried_peers = self.context.table.already_tried_peers.lock().await;
        let peer_number = self.context.table.peers.lock().await.len() as u64;

        if self.last_log_time.elapsed() > self.lookup_interval {
            info!(
                "Resetting list of tried peers. Current peers {}",
                peer_number,
            );
            self.last_log_time = Instant::now();
            already_tried_peers.clear();
        }

        if peer_number > self.target_peers {
            return false;
        }

        for contact in self.context.table.table.lock().await.values() {
            let node_id = contact.node.node_id();
            if !already_tried_peers.contains(&node_id) && contact.knows_us {
                already_tried_peers.insert(node_id);

                RLPxConnection::spawn_as_initiator(self.context.clone(), &contact.node).await;

                METRICS.record_new_rlpx_conn_attempt().await;
                return true;
            }
        }
        false
    }

    async fn get_lookup_interval(&self) -> Duration {
        let num_peers = self.context.table.peers.lock().await.len() as u64;

        if num_peers < self.target_peers {
            self.initial_lookup_interval
        } else {
            info!("Reached target number of peers. Using longer lookup interval.");
            self.lookup_interval
        }
    }

    pub async fn down(handle: &mut GenServerHandle<RLPxInitiator>) {
        handle.cast(InMessage::LookForPeer).await;
    }
}

#[derive(Debug, Clone)]
pub enum InMessage {
    LookForPeer,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

impl GenServer for RLPxInitiator {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = RLPxInitiatorError;

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            Self::CastMsg::LookForPeer => {
                debug!(received = "Look for peers");

                if !self.look_for_peer().await {
                    send_after(
                        self.get_lookup_interval().await,
                        handle.clone(),
                        Self::CastMsg::LookForPeer,
                    );
                };

                CastResponse::NoReply
            }
        }
    }
}

pub static INITIATOR: LazyLock<OnceCell<GenServerHandle<RLPxInitiator>>> =
    LazyLock::new(OnceCell::new);
