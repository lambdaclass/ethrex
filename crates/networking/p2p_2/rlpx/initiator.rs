use std::{fs::read_to_string, str::FromStr, sync::Arc, time::Duration};

use ethrex_common::{H256, types::ForkId};
use k256::ecdsa::SigningKey;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, send_after},
};
use tokio::sync::Mutex;
use tracing::{debug, info};

use crate::{
    kademlia::Kademlia,
    metrics::METRICS,
    network::P2PContext,
    types::{Node, NodeRecord},
};

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
    local_node: Node,
    local_node_record: Arc<Mutex<NodeRecord>>,
    signer: SigningKey,
    // udp_socket: Arc<UdpSocket>,
    initial_lookup_period: Duration,
    lookup_period: Duration,
    // lookup_period: Duration,
    kademlia: Kademlia,
    /// The target number of RLPx connections to reach.
    target_peers: usize,
    /// The limit on the number of tried connections.
    limit_tried_peers: usize,
}

impl RLPxInitiatorState {
    pub fn new(
        context: P2PContext,
        local_node: Node,
        local_node_record: Arc<Mutex<NodeRecord>>,
        signer: SigningKey,
        // udp_socket: Arc<UdpSocket>,
        kademlia: Kademlia,
    ) -> Self {
        let _geth_peers = serde_json::from_str::<Vec<String>>(
            &read_to_string("geth_peers.json").expect("Failed to read geth_peers.json"),
        )
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
            local_node,
            local_node_record,
            signer,
            // udp_socket,
            kademlia,
            initial_lookup_period: Duration::from_secs(3),
            lookup_period: Duration::from_secs(60),
            target_peers: 50,
            limit_tried_peers: 50_000,
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
    pub async fn spawn(
        context: P2PContext,
        local_node: Node,
        signer: SigningKey,
        fork_id: &ForkId,
        // udp_socket: Arc<UdpSocket>,
        kademlia: Kademlia,
    ) -> Result<(), RLPxInitiatorError> {
        info!("Starting RLPx Initiator");

        let local_node_record = Arc::new(Mutex::new(
            NodeRecord::from_node(&local_node, 1, &signer, fork_id)
                .expect("Failed to create local node record"),
        ));

        let state =
            RLPxInitiatorState::new(context, local_node, local_node_record, signer, kademlia);

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
                    get_lookup_interval(&state).await,
                    handle.clone(),
                    Self::CastMsg::LookForPeers,
                );

                CastResponse::NoReply(state)
            }
        }
    }
}

async fn get_lookup_interval(state: &RLPxInitiatorState) -> Duration {
    let num_peers = state.kademlia.table.lock().await.len();
    let num_tried_peers = state.kademlia.already_tried_peers.lock().await.len();

    if num_peers < state.target_peers && num_tried_peers < state.limit_tried_peers {
        state.initial_lookup_period
    } else {
        state.lookup_period
    }
}

async fn look_for_peers(state: &RLPxInitiatorState) {
    info!("Looking for peers");

    let peers = state.kademlia.table.lock().await;
    let mut already_tried_peers_table = state.kademlia.already_tried_peers.lock().await;

    for contact in peers.values() {
        let node_id = contact.node.node_id();
        if !already_tried_peers_table.contains(&node_id) && contact.knows_us {
            already_tried_peers_table.insert(node_id);

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
