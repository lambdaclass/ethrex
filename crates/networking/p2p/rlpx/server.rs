use std::time::Duration;

use spawned_concurrency::{
    error::GenServerError,
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, send_after},
};
use tracing::{error, info};

use crate::{
    discv4::server::Discv4Server,
    kademlia::PeerChannels,
    rlpx::{connection::server::RLPxConnection, lookup::RLPxLookupServer},
    types::Node,
};

const MAX_PEER_COUNT: usize = 50;
const MAX_CONCURRENT_LOOKUPS: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum RLPxServerError {}

#[derive(Debug, Clone)]
pub struct RLPxServerState {
    discovery_server: Discv4Server,
    lookup_servers: Vec<GenServerHandle<RLPxLookupServer>>,
    connections: Vec<PeerChannels>,
}

#[derive(Clone)]
pub enum InMessage {
    NewPeer(Node),
    BookKeeping,
}

#[derive(Clone, PartialEq)]
pub enum OutMessage {}

#[derive(Debug, Clone)]
pub struct RLPxServer;

impl RLPxServer {
    pub async fn spawn(
        discovery_server: Discv4Server,
    ) -> Result<GenServerHandle<Self>, GenServerError> {
        let state = RLPxServerState {
            discovery_server,
            lookup_servers: vec![],
            connections: vec![],
        };
        // TODO: spawn multiple lookup servers
        let mut handle = Self::start(state);
        handle.cast(InMessage::BookKeeping).await?;
        Ok(handle)
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
        msg: Self::CastMsg,
        handle: &GenServerHandle<Self>,
        mut state: Self::State,
    ) -> CastResponse<Self> {
        match msg {
            InMessage::NewPeer(node) => {
                info!("Found new peer: {node}");
                // start_new_connection
                RLPxConnection::spawn_as_initiator(state.discovery_server, &node);
                state.connections.append(node);
            }
            InMessage::BookKeeping => {
                info!("Performing bookkeeping");
                bookkeeping(handle, &mut state).await;
            }
        }
        CastResponse::NoReply(state)
    }
}

/// Perform periodic tasks
async fn bookkeeping(handle: &GenServerHandle<RLPxServer>, state: &mut RLPxServerState) {
    if state.connections.len() >= MAX_PEER_COUNT
        || state.lookup_servers.len() >= MAX_CONCURRENT_LOOKUPS
    {
        return;
    }
    info!("Spawning new lookup servers");

    for _ in 0..MAX_CONCURRENT_LOOKUPS {
        let node_iterator = state.discovery_server.new_random_iterator();
        let Ok(new_lookup_server) = RLPxLookupServer::spawn(node_iterator, handle.clone())
            .await
            .inspect_err(|e| error!("Failed to spawn lookup server: {e}"))
        else {
            continue;
        };
        state.lookup_servers.push(new_lookup_server);
    }
    send_after(
        Duration::from_secs(5),
        handle.clone(),
        InMessage::BookKeeping,
    );
}
