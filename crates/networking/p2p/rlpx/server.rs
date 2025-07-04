use std::time::Duration;

use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, send_after},
};

use crate::{discv4::server::Discv4NodeIterator, kademlia::PeerChannels};

#[derive(Debug, thiserror::Error)]
pub enum RLPxServerError {}

#[derive(Debug, Clone)]
pub struct RLPxServerState {
    node_iterator: Discv4NodeIterator,
    connections: Vec<PeerChannels>,
}

#[derive(Clone)]
pub enum InMessage {
    FetchPeers,
}

#[derive(Clone, PartialEq)]
pub enum OutMessage {}

pub struct RLPxServer;

impl RLPxServer {
    pub fn spawn(node_iterator: Discv4NodeIterator) -> GenServerHandle<Self> {
        let state = RLPxServerState {
            node_iterator,
            connections: vec![],
        };
        let handle = Self::start(state);
        send_after(
            Duration::from_millis(100),
            handle.clone(),
            InMessage::FetchPeers,
        );
        handle
    }
}

impl GenServer for RLPxServer {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type State = RLPxServerState;
    type Error = RLPxServerError;

    fn new() -> Self {
        Self
    }

    async fn handle_cast(
        &mut self,
        InMessage::FetchPeers: Self::CastMsg,
        handle: &GenServerHandle<Self>,
        mut state: Self::State,
    ) -> CastResponse<Self> {
        send_after(
            Duration::from_millis(10),
            handle.clone(),
            InMessage::FetchPeers,
        );
        let node = state.node_iterator.next().await;
        CastResponse::NoReply(state)
    }
}
