use std::time::Duration;

use spawned_concurrency::{
    error::GenServerError,
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, send_after},
};
use tracing::{error, info};

use crate::{
    discv4::server::Discv4Server,
    network::P2PContext,
    rlpx::{
        connection::server::RLPxConnection,
        lookup::RLPxLookupServer,
        p2p::{
            SUPPORTED_ETH_CAPABILITIES, SUPPORTED_P2P_CAPABILITIES, SUPPORTED_SNAP_CAPABILITIES,
        },
    },
    types::{Node, NodeRecord},
};

const MAX_PEER_COUNT: usize = 50;
const MAX_CONCURRENT_LOOKUPS: usize = 1;

pub type RLPxServerHandle = GenServerHandle<RLPxServer>;

#[derive(Debug, Clone)]
pub struct RLPxServerState {
    ctx: P2PContext,
    discovery_server: Discv4Server,
    lookup_servers: Vec<GenServerHandle<RLPxLookupServer>>,
}

#[derive(Clone)]
pub enum InMessage {
    BookKeeping,
}

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
    type OutMsg = Unused;
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
        match msg {
            InMessage::BookKeeping => {
                bookkeeping(handle, &mut state).await;
            }
        }
        CastResponse::NoReply(state)
    }
}

/// Perform periodic tasks
async fn bookkeeping(handle: &GenServerHandle<RLPxServer>, state: &mut RLPxServerState) {
    send_after(
        Duration::from_secs(5),
        handle.clone(),
        InMessage::BookKeeping,
    );

    {
        let mut table_lock = state.ctx.table.lock().await;
        let nodes_without_enr: Vec<_> = table_lock
            .iter_peers()
            .filter(|p| p.record == NodeRecord::default() && p.enr_request_hash.is_none())
            .map(|p| p.node.clone())
            .take(128)
            .collect();
        for node in nodes_without_enr {
            let _ = state
                .discovery_server
                .send_enr_request(&node, &mut table_lock)
                .await;
        }

        let nodes_without_connection: Vec<_> = table_lock
            .iter_peers()
            .filter(|p| {
                !p.is_connected && p.channels.is_none() && p.record != NodeRecord::default()
            })
            .map(|p| p.node.clone())
            .take(128)
            .collect();
        for node in nodes_without_connection {
            RLPxConnection::spawn_as_initiator(state.ctx.clone(), &node).await;
        }
    }

    // Stop looking for peers if we have enough connections
    if got_enough_peers(state).await {
        stop_lookup_servers(state).await;
    // Otherwise, spawn the lookup servers
    } else if state.lookup_servers.is_empty() {
        info!("Spawning new lookup servers");
        spawn_lookup_servers(state, handle).await;
    }
}

async fn spawn_lookup_servers(state: &mut RLPxServerState, handle: &GenServerHandle<RLPxServer>) {
    let known_node = Node::from_enode_url(
        "enode://da1568823fdfccdcc37de2f3751987510c055ac24240cb8261aab5e3510b5e6222083d70a99ee29fa971a0646b37565311d31497054d6bdf320f8b3ea20749b6@177.54.155.141:60300",
    ).unwrap();
    RLPxConnection::spawn_as_initiator(state.ctx.clone(), &known_node).await;

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
}

async fn stop_lookup_servers(state: &mut RLPxServerState) {
    for mut server in state.lookup_servers.drain(..) {
        let _ = RLPxLookupServer::stop(&mut server)
            .await
            .inspect_err(|e| error!("Failed to stop lookup server: {e}"));
    }
}

async fn got_enough_peers(state: &RLPxServerState) -> bool {
    let table = state.ctx.table.lock().await;
    // Check we have a good amount of peers that support p2p+eth+snap
    let snap_peers = table
        .iter_peers()
        .filter(|peer| {
            peer.is_connected
                && peer
                    .supported_capabilities
                    .iter()
                    .any(|c| SUPPORTED_SNAP_CAPABILITIES.contains(c))
                && peer
                    .supported_capabilities
                    .iter()
                    .any(|c| SUPPORTED_ETH_CAPABILITIES.contains(c))
                && peer
                    .supported_capabilities
                    .iter()
                    .any(|c| SUPPORTED_P2P_CAPABILITIES.contains(c))
        })
        .count();

    snap_peers >= MAX_PEER_COUNT
}
