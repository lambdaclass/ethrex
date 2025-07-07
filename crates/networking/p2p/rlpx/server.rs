use std::time::Duration;

use spawned_concurrency::{
    error::GenServerError,
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, send_after},
};
use tracing::{error, info};

use crate::{
    discv4::server::Discv4Server, network::P2PContext, rlpx::lookup::RLPxLookupServer, types::Node,
};

const MAX_PEER_COUNT: usize = 50;
const MAX_CONCURRENT_LOOKUPS: usize = 4;

pub type RLPxServerHandle = GenServerHandle<RLPxServer>;

#[derive(Debug, Clone)]
pub struct RLPxServerState {
    ctx: P2PContext,
    discovery_server: Discv4Server,
    lookup_servers: Vec<GenServerHandle<RLPxLookupServer>>,
    connections: Vec<Node>,
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
        ctx: P2PContext,
        discovery_server: Discv4Server,
    ) -> Result<GenServerHandle<Self>, GenServerError> {
        let state = RLPxServerState {
            ctx,
            discovery_server,
            lookup_servers: vec![],
            connections: vec![],
        };
        // TODO: spawn multiple lookup servers
        let mut handle = Self::start(state);
        handle.cast(InMessage::BookKeeping).await?;
        Ok(handle)
    }

    pub async fn add_peer(handle: &mut RLPxServerHandle, peer: Node) -> Result<(), GenServerError> {
        handle.cast(InMessage::NewPeer(peer)).await
    }
}

impl GenServer for RLPxServer {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type State = RLPxServerState;
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
        info!("Received cast message");
        match msg {
            InMessage::NewPeer(node) => {
                state.connections.push(node);
            }
            InMessage::BookKeeping => {
                bookkeeping(handle, &mut state).await;
            }
        }
        CastResponse::NoReply(state)
    }
}

/// Perform periodic tasks
async fn bookkeeping(handle: &GenServerHandle<RLPxServer>, state: &mut RLPxServerState) {
    if state.connections.len() >= MAX_PEER_COUNT {
        for mut server in state.lookup_servers.drain(..) {
            let _ = RLPxLookupServer::stop(&mut server)
                .await
                .inspect_err(|e| error!("Failed to stop lookup server: {e}"));
        }
    }
    info!("Spawning new lookup servers");

    for _ in 0..MAX_CONCURRENT_LOOKUPS {
        let node_iterator = state.discovery_server.new_random_iterator();
        let Ok(new_lookup_server) =
            RLPxLookupServer::spawn(state.ctx.clone(), node_iterator, handle.clone())
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
