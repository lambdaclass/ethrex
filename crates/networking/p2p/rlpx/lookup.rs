use spawned_concurrency::{
    error::GenServerError,
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle},
};
use tracing::error;

use crate::{
    discv4::server::Discv4NodeIterator,
    network::P2PContext,
    rlpx::{
        connection::server::RLPxConnection,
        server::{RLPxServer, RLPxServerHandle},
    },
};

pub type RLPxLookupServerHandle = GenServerHandle<RLPxLookupServer>;

#[derive(Debug, Clone)]
pub struct RLPxLookupServerState {
    ctx: P2PContext,
    node_iterator: Discv4NodeIterator,
    consumer: RLPxServerHandle,
}

#[derive(Debug, Clone)]
pub struct RLPxLookupServer;

impl RLPxLookupServer {
    pub async fn spawn(
        ctx: P2PContext,
        node_iterator: Discv4NodeIterator,
        consumer: GenServerHandle<RLPxServer>,
    ) -> Result<GenServerHandle<Self>, GenServerError> {
        let state = RLPxLookupServerState {
            ctx,
            node_iterator,
            consumer,
        };
        let mut handle = Self::start(state);
        handle.cast(InMessage::FetchPeers).await?;
        Ok(handle)
    }

    pub async fn stop(handle: &mut RLPxLookupServerHandle) -> Result<(), GenServerError> {
        handle.cast(InMessage::Stop).await
    }
}

#[derive(Debug, Clone)]
pub enum InMessage {
    FetchPeers,
    Stop,
}

impl GenServer for RLPxLookupServer {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = Unused;
    type State = RLPxLookupServerState;
    type Error = std::convert::Infallible;

    fn new() -> Self {
        Self
    }

    async fn handle_cast(
        &mut self,
        msg: Self::CastMsg,
        handle: &GenServerHandle<Self>,
        mut state: Self::State,
    ) -> CastResponse<Self> {
        if matches!(msg, InMessage::Stop) {
            return CastResponse::Stop;
        }
        // Stop on error
        if handle.clone().cast(InMessage::FetchPeers).await.is_err() {
            error!("RLPxLookupServer: failed to send message to self, stopping lookup");
            return CastResponse::Stop;
        }
        let node = state.node_iterator.next().await;
        let node_id = node.node_id();
        // Get peer status and mark as connected
        {
            let mut table = state.ctx.table.lock().await;
            table.insert_node_forced(node.clone());
            let node = table
                .get_by_node_id_mut(node_id)
                .expect("we just inserted this node");

            // If we already have a connection to this node, we don't need to start a new one
            if node.is_connected {
                drop(table);
                return CastResponse::NoReply(state);
            }
            node.is_connected = true;
        }
        // Start a connection
        RLPxConnection::spawn_as_initiator(state.ctx.clone(), &node).await;

        if RLPxServer::add_peer(&mut state.consumer, node)
            .await
            .is_err()
        {
            error!("RLPxLookupServer: failed to send message to consumer, stopping lookup");
            return CastResponse::Stop;
        }
        CastResponse::NoReply(state)
    }
}
