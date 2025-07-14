use std::{sync::Arc, time::Duration};

use ethrex_common::H32;
use k256::ecdsa::SigningKey;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, send_after},
};
use tokio::sync::Mutex;
use tracing::{debug, info};

use crate::{
    discv4::Kademlia,
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
    context: P2PContext,
    local_node: Node,
    local_node_record: Arc<Mutex<NodeRecord>>,
    signer: SigningKey,
    // udp_socket: Arc<UdpSocket>,
    lookup_period: Duration,
    // lookup_period: Duration,
    kademlia: Kademlia,
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
        Self {
            context,
            local_node,
            local_node_record,
            signer,
            // udp_socket,
            kademlia,

            lookup_period: Duration::from_secs(3),
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
        // udp_socket: Arc<UdpSocket>,
        kademlia: Kademlia,
    ) -> Result<(), RLPxInitiatorError> {
        info!("Starting RLPx Initiator");

        let local_node_record = Arc::new(Mutex::new(
            NodeRecord::from_node(&local_node, 1, &signer)
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
                    state.lookup_period,
                    handle.clone(),
                    Self::CastMsg::LookForPeers,
                );

                CastResponse::NoReply(state)
            }
        }
    }
}

async fn look_for_peers(state: &RLPxInitiatorState) {
    const ACCEPTED_FORK_HASHES: [H32; 4] = [
        H32([0xc6, 0x1a, 0x60, 0x98]),
        H32([0xfd, 0x4f, 0x01, 0x6b]),
        H32([0x9b, 0x19, 0x2a, 0xd0]),
        H32([0xdf, 0xbd, 0x9b, 0xed]),
    ];
    info!("Looking for peers");
    let mut already_known_peers_table = state.kademlia.already_tried_peers.lock().await;
    let mut invalid_fork_ids = 0;
    let mut no_fork_ids = 0;
    let mut connected_peers = 0;
    for node in state.kademlia.table.lock().await.values() {
        let Some(fork_id) = &node.fork_id else {
            no_fork_ids += 1;
            continue;
        };

        if !ACCEPTED_FORK_HASHES.contains(&fork_id.fork_hash) {
            invalid_fork_ids += 1;
            continue;
        }
        if !already_known_peers_table.contains(&node.node_id()) {
            already_known_peers_table.insert(node.node_id());
            RLPxConnection::spawn_as_initiator(state.context.clone(), node).await;
            connected_peers += 1;
        }
    }

    info!(
        invalid_fork_ids = invalid_fork_ids,
        no_fork_ids = no_fork_ids,
        connected_peers = connected_peers,
    );
}
