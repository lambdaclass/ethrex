use std::{fs::read_to_string, net::SocketAddr, str::FromStr, sync::Arc, time::Duration};

use ethrex_common::{H256, types::ForkId};
use k256::{ecdsa::SigningKey,};
use rand::rngs::OsRng;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, send_after},
};
use tokio::{net::UdpSocket, sync::Mutex};
use tracing::{debug, error, info};

use crate::{
    discv4::messages::{FindNodeMessage, Message, PingMessage},
    kademlia::Kademlia,
    metrics::METRICS,
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
    _geth_peers: Vec<H256>,
    local_node: Node,
    local_node_record: Arc<Mutex<NodeRecord>>,
    signer: SigningKey,
    udp_socket: Arc<UdpSocket>,

    revalidation_period: Duration,
    lookup_period: Duration,
    prune_period: Duration,

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
        let _geth_peers = serde_json::from_str::<Vec<String>>(
            &read_to_string("/Users/ivanlitteri/Repositories/lambdaclass/ethrex/geth_peers.json")
                .expect("Failed to read geth_peers.json"),
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
            local_node,
            local_node_record,
            signer,
            udp_socket,
            kademlia,

            revalidation_period: Duration::from_secs(5),
            lookup_period: Duration::from_secs(5),
            prune_period: Duration::from_secs(5),
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

        let random_priv_key = SigningKey::random(&mut OsRng);
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
        fork_id: &ForkId,
        udp_socket: Arc<UdpSocket>,
        kademlia: Kademlia,
    ) -> Result<(), DiscoverySideCarError> {
        info!("Starting Discovery Side Car");

        let local_node_record = Arc::new(Mutex::new(
            NodeRecord::from_node(&local_node, 1, &signer, fork_id)
                .expect("Failed to create local node record"),
        ));

        let state =
            DiscoverySideCarState::new(local_node, local_node_record, signer, udp_socket, kademlia);

        let mut server = DiscoverySideCar::start(state.clone());

        let _ = server.cast(InMessage::Revalidate).await;

        let _ = server.cast(InMessage::Lookup).await;

        let _ = server.cast(InMessage::Prune).await;

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

                prune(&state).await;

                send_after(state.prune_period, handle.clone(), Self::CastMsg::Prune);

                CastResponse::NoReply(state)
            }
        }
    }
}

async fn revalidate(state: &DiscoverySideCarState) {
    for contact in state.kademlia.table.lock().await.values_mut() {
        if contact.disposable {
            continue;
        }

        let node_id = contact.node.node_id();

        match state.ping(&contact.node).await {
            Ok(_) => {
                if state._geth_peers.contains(&node_id) {
                    METRICS.new_pinged_mainnet_peer(node_id).await;
                }
            }
            Err(err) => {
                error!(sent = "Ping", to = %format!("{:#x}", contact.node.public_key), err = ?err);

                contact.disposable = true;

                METRICS.record_discarded_contact().await;

                if state._geth_peers.contains(&node_id) {
                    METRICS.new_failure_pinging_mainnet_peer(node_id).await;
                }
            }
        }
    }
}

async fn lookup(state: &DiscoverySideCarState) {
    for contact in state.kademlia.table.lock().await.values_mut() {
        if contact.n_find_node_sent == 20 || contact.disposable {
            continue;
        }

        if let Err(err) = state.send_find_node(&contact.node).await {
            error!(sent = "FindNode", to = %format!("{:#x}", contact.node.public_key), err = ?err);
            contact.disposable = true;
            METRICS.record_discarded_contact().await;
        }

        contact.n_find_node_sent += 1;
    }
}

async fn prune(state: &DiscoverySideCarState) {
    let mut contacts = state.kademlia.table.lock().await;
    let mut discarded_contacts = state.kademlia.discarded_contacts.lock().await;

    let disposable_contacts = contacts
        .iter()
        .filter_map(|(c_id, c)| c.disposable.then_some(*c_id))
        .collect::<Vec<_>>();

    for contact_to_discard_id in disposable_contacts {
        contacts.remove(&contact_to_discard_id);
        discarded_contacts.insert(contact_to_discard_id);
    }
}
