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
        server::{InMessage, RLPxServer},
    },
};

#[derive(Debug, Clone)]
pub struct RLPxLookupServerState {
    ctx: P2PContext,
    node_iterator: Discv4NodeIterator,
    consumer: GenServerHandle<RLPxServer>,
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
        handle.cast(FetchPeers).await?;
        Ok(handle)
    }
}

#[derive(Debug, Clone)]
pub struct FetchPeers;

impl GenServer for RLPxLookupServer {
    type CallMsg = Unused;
    type CastMsg = FetchPeers;
    type OutMsg = Unused;
    type State = RLPxLookupServerState;
    type Error = std::convert::Infallible;

    fn new() -> Self {
        Self
    }

    async fn handle_cast(
        &mut self,
        _msg: Self::CastMsg,
        handle: &GenServerHandle<Self>,
        mut state: Self::State,
    ) -> CastResponse<Self> {
        // Stop on error
        if handle.clone().cast(FetchPeers).await.is_err() {
            error!("RLPxLookupServer: failed to send message to self, stopping lookup");
            return CastResponse::Stop;
        }
        let node = state.node_iterator.next().await;

        // Start a connection
        RLPxConnection::spawn_as_initiator(state.ctx.clone(), &node).await;

        if state.consumer.cast(InMessage::NewPeer(node)).await.is_err() {
            error!("RLPxLookupServer: failed to send message to consumer, stopping lookup");
            return CastResponse::Stop;
        }
        CastResponse::NoReply(state)
    }
}
