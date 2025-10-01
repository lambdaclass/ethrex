use std::time::Duration;

use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, send_after},
};

use tracing::{debug, error, info};

use crate::{discv4::peer_table::PeerTableError, metrics::METRICS, network::P2PContext};

use crate::rlpx::connection::server::RLPxConnection;

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
    /// The rate at which to try new connections.
    new_connections_per_lookup: usize,
}

impl RLPxInitiator {
    pub fn new(context: P2PContext) -> Self {
        Self {
            context,
            initial_lookup_interval: Duration::from_secs(3),
            lookup_interval: Duration::from_secs(5 * 60),
            target_peers: 50,
            new_connections_per_lookup: 5000,
        }
    }

    pub async fn spawn(context: P2PContext) {
        info!("Starting RLPx Initiator");

        let state = RLPxInitiator::new(context);

        let mut server = RLPxInitiator::start(state.clone());

        let _ = server.cast(InMessage::LookForPeers).await;
    }

    async fn look_for_peers(&mut self) -> Result<(), RLPxInitiatorError> {
        info!("Looking for peers");

        let contacts = self
            .context
            .table
            .get_contacts_to_initiate(self.new_connections_per_lookup)
            .await?;

        for contact in contacts {
            RLPxConnection::spawn_as_initiator(self.context.clone(), &contact.node).await;
            METRICS.record_new_rlpx_conn_attempt().await;
        }
        Ok(())
    }

    async fn get_lookup_interval(&mut self) -> Duration {
        let num_peers = self.context.table.peer_count().await.unwrap_or(0) as u64;

        if num_peers < self.target_peers {
            self.initial_lookup_interval
        } else {
            info!("Reached target number of peers. Using longer lookup interval.");
            self.lookup_interval
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

impl GenServer for RLPxInitiator {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = std::convert::Infallible;

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            Self::CastMsg::LookForPeers => {
                debug!(received = "Look for peers");

                let _ = self
                    .look_for_peers()
                    .await
                    .inspect_err(|e| error!(err=?e, "Error looking for peers"));

                send_after(
                    self.get_lookup_interval().await,
                    handle.clone(),
                    Self::CastMsg::LookForPeers,
                );

                CastResponse::NoReply
            }
        }
    }
}
