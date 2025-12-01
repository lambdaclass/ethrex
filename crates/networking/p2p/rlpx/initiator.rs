use crate::types::Node;
use crate::{
    discv4::{
        peer_table::PeerTableError,
        server::{INITIAL_LOOKUP_INTERVAL, LOOKUP_INTERVAL},
    },
    metrics::METRICS,
    network::P2PContext,
    rlpx::connection::server::PeerConnection,
};
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, InitResult, send_after, send_message_on},
};
use std::time::Duration;
use tracing::{debug, error, info};

#[derive(Debug, thiserror::Error)]
pub enum RLPxInitiatorError {
    #[error(transparent)]
    PeerTableError(#[from] PeerTableError),
}

#[derive(Debug, Clone)]
pub struct RLPxInitiator {
    context: P2PContext,

    /// The initial interval between peer lookups, until the number of peers
    /// reaches [target_peers](RLPxInitiatorState::target_peers).
    initial_lookup_interval: Duration,
    lookup_interval: Duration,

    /// The target number of RLPx connections to reach.
    target_peers: u64,
}

impl RLPxInitiator {
    pub fn new(context: P2PContext) -> Self {
        Self {
            context,
            // We use the same lookup intervals as Discovery to try to get both process to check at the same rate
            initial_lookup_interval: INITIAL_LOOKUP_INTERVAL,
            lookup_interval: LOOKUP_INTERVAL,
            target_peers: 50,
        }
    }

    pub async fn spawn(context: P2PContext) -> GenServerHandle<RLPxInitiator> {
        info!("Starting RLPx Initiator");
        let state = RLPxInitiator::new(context);
        let mut server = RLPxInitiator::start(state.clone());
        let _ = server.cast(InMessage::LookForPeer).await;
        server
    }

    async fn look_for_peer(&mut self) -> Result<(), RLPxInitiatorError> {
        if !self.context.table.target_peers_reached().await? {
            if let Some(contact) = self.context.table.get_contact_to_initiate().await? {
                PeerConnection::spawn_as_initiator(self.context.clone(), &contact.node).await;
                METRICS.record_new_rlpx_conn_attempt().await;
            };
        } else {
            debug!("Target peer connections reached, no need to initiate new connections.");
        }
        Ok(())
    }

    async fn get_lookup_interval(&mut self) -> Duration {
        let num_peers = self.context.table.peer_count().await.unwrap_or(0) as u64;

        if num_peers < self.target_peers {
            self.initial_lookup_interval
        } else {
            debug!("Reached target number of peers. Using longer lookup interval.");
            self.lookup_interval
        }
    }
}

#[derive(Debug, Clone)]
pub enum InMessage {
    LookForPeer,
    Initiate { node: Node },
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

impl GenServer for RLPxInitiator {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = std::convert::Infallible;

    async fn init(self, handle: &GenServerHandle<Self>) -> Result<InitResult<Self>, Self::Error> {
        send_message_on(handle.clone(), tokio::signal::ctrl_c(), InMessage::Shutdown);
        Ok(InitResult::Success(self))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            Self::CastMsg::LookForPeer => {
                let _ = self
                    .look_for_peer()
                    .await
                    .inspect_err(|e| error!(err=?e, "Error looking for peers"));

                send_after(
                    self.get_lookup_interval().await,
                    handle.clone(),
                    Self::CastMsg::LookForPeer,
                );

                CastResponse::NoReply
            }
            Self::CastMsg::Initiate { node } => {
                PeerConnection::spawn_as_initiator(self.context.clone(), &node).await;
                METRICS.record_new_rlpx_conn_attempt().await;
                CastResponse::NoReply
            }
            Self::CastMsg::Shutdown => CastResponse::Stop,
        }
    }
}
