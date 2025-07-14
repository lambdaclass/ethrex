use std::{net::SocketAddr, sync::Arc, time::Duration};

use ethrex_common::H512;
use k256::{PublicKey, ecdsa::SigningKey, elliptic_curve::sec1::ToEncodedPoint};
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, send_after},
};
use tokio::{net::UdpSocket, sync::Mutex};
use tracing::{debug, error, info};

use crate::{
    discv4::{
        Kademlia,
        messages::{ENRRequestMessage, FindNodeMessage, Message, PingMessage},
    },
    types::{Endpoint, Node, NodeRecord},
    utils::{get_msg_expiration_from_seconds, public_key_from_signing_key},
};

#[derive(Debug, thiserror::Error)]
pub enum DiscoverySideCarError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("Failed to send message")]
    MessageSendFailure(std::io::Error),
    #[error("Only partial message was sent")]
    PartialMessageSent,
}

#[derive(Debug, Clone)]
pub struct DiscoverySideCarState {
    local_node: Node,
    local_node_record: Arc<Mutex<NodeRecord>>,
    signer: SigningKey,
    udp_socket: Arc<UdpSocket>,

    revalidation_period: Duration,
    lookup_period: Duration,

    kademlia: Kademlia,
}

impl DiscoverySideCarState {
    pub fn new(
        local_node: Node,
        local_node_record: Arc<Mutex<NodeRecord>>,
        signer: SigningKey,
        udp_socket: Arc<UdpSocket>,
        kademlia: Kademlia,
    ) -> Self {
        Self {
            local_node,
            local_node_record,
            signer,
            udp_socket,
            kademlia,

            revalidation_period: Duration::from_secs(5),
            lookup_period: Duration::from_secs(5),
        }
    }

    async fn ping(&self, node: &Node) -> Result<(), DiscoverySideCarError> {
        let mut buf = Vec::new();

        // TODO: Parametrize this expiration.
        let expiration: u64 = get_msg_expiration_from_seconds(20);

        let from = Endpoint {
            ip: self.local_node.ip,
            udp_port: self.local_node.udp_port,
            tcp_port: self.local_node.tcp_port,
        };

        let to = Endpoint {
            ip: node.ip,
            udp_port: node.udp_port,
            tcp_port: node.tcp_port,
        };

        let enr_seq = self.local_node_record.lock().await.seq;

        let ping = Message::Ping(PingMessage::new(from, to, expiration).with_enr_seq(enr_seq));

        ping.encode_with_header(&mut buf, &self.signer);

        let bytes_sent = self
            .udp_socket
            .send_to(&buf, node.udp_addr())
            .await
            .map_err(DiscoverySideCarError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            return Err(DiscoverySideCarError::PartialMessageSent);
        }

        debug!(sent = "Ping", to = %format!("{:#x}", node.public_key));

        Ok(())
    }

    async fn send_find_node(&self, node: &Node) -> Result<(), DiscoverySideCarError> {
        let expiration: u64 = get_msg_expiration_from_seconds(20);

        let random_priv_key = SigningKey::random(&mut rand::rngs::OsRng);
        let random_pub_key = public_key_from_signing_key(&random_priv_key);

        let msg = Message::FindNode(FindNodeMessage::new(random_pub_key, expiration));

        let mut buf = Vec::new();
        msg.encode_with_header(&mut buf, &self.signer);
        let bytes_sent = self
            .udp_socket
            .send_to(&buf, SocketAddr::new(node.ip, node.udp_port))
            .await
            .map_err(DiscoverySideCarError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            return Err(DiscoverySideCarError::PartialMessageSent);
        }

        debug!(sent = "FindNode", to = %format!("{:#x}", node.public_key));

        Ok(())
    }

    async fn send_enr_request(&self, node: &Node) -> Result<(), DiscoverySideCarError> {
        let mut buf = Vec::new();

        // TODO: Parametrize this expiration.
        let expiration: u64 = get_msg_expiration_from_seconds(20);

        let enr_req = Message::ENRRequest(ENRRequestMessage::new(expiration));

        enr_req.encode_with_header(&mut buf, &self.signer);

        let bytes_sent = self
            .udp_socket
            .send_to(&buf, node.udp_addr())
            .await
            .map_err(DiscoverySideCarError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            return Err(DiscoverySideCarError::PartialMessageSent);
        }

        debug!(sent = "ENRRequest", to = %format!("{:#x}", node.public_key));

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum InMessage {
    Revalidate,
    Lookup,
    Prune,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

pub struct DiscoverySideCar;

impl DiscoverySideCar {
    pub async fn spawn(
        local_node: Node,
        signer: SigningKey,
        udp_socket: Arc<UdpSocket>,
        kademlia: Kademlia,
    ) -> Result<(), DiscoverySideCarError> {
        info!("Starting Discovery Side Car");

        let local_node_record = Arc::new(Mutex::new(
            NodeRecord::from_node(&local_node, 1, &signer)
                .expect("Failed to create local node record"),
        ));

        let state =
            DiscoverySideCarState::new(local_node, local_node_record, signer, udp_socket, kademlia);

        let mut server = DiscoverySideCar::start(state.clone());

        let _ = server.cast(InMessage::Revalidate).await;

        let _ = server.cast(InMessage::Lookup).await;

        Ok(())
    }
}

impl GenServer for DiscoverySideCar {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type State = DiscoverySideCarState;
    type Error = DiscoverySideCarError;

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
            Self::CastMsg::Revalidate => {
                debug!(received = "Revalidate");

                revalidate(&state).await;

                send_after(
                    state.revalidation_period,
                    handle.clone(),
                    Self::CastMsg::Revalidate,
                );

                CastResponse::NoReply(state)
            }
            Self::CastMsg::Lookup => {
                debug!(received = "Lookup");

                lookup(&state).await;

                send_after(state.lookup_period, handle.clone(), Self::CastMsg::Lookup);

                CastResponse::NoReply(state)
            }
            Self::CastMsg::Prune => {
                debug!(received = "Prune");

                // Once we have a pruning strategy, we can implement it here.
                // For now, no one is pruned.
                CastResponse::NoReply(state)
            }
        }
    }
}

async fn revalidate(state: &DiscoverySideCarState) {
    for node in state.kademlia.contacts.lock().await.values() {
        let _ = state.ping(node).await.inspect_err(
            |e| error!(sent = "Ping", to = %format!("{:#x}", node.public_key), err = ?e),
        );
    }
}

async fn lookup(state: &DiscoverySideCarState) {
    {
        if state.kademlia.contacts.lock().await.len() >= 18000 {
            return;
        }
    }
    for node in state.kademlia.contacts.lock().await.values() {
        let _ = state.send_find_node(node).await.inspect_err(
            |e| error!(sent = "FindNode", to = %format!("{:#x}", node.public_key), err = ?e),
        );
    }
}

async fn prune(state: &DiscoverySideCarState) {
    // Remove nodes tagged as disposable.
}
