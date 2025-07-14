use std::{collections::hash_map::Entry, net::SocketAddr, sync::Arc};

use ethrex_common::H512;
use k256::ecdsa::SigningKey;
use keccak_hash::H256;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle},
};
use tokio::{net::UdpSocket, sync::Mutex};
use tracing::{debug, error, info, warn};

use crate::{
    discv4::{
        Kademlia,
        messages::{
            ENRRequestMessage, ENRResponseMessage, FindNodeMessage, Message, NeighborsMessage,
            Packet, PacketDecodeErr, PingMessage, PongMessage,
        },
        metrics::METRICS,
    },
    types::{Endpoint, Node, NodeRecord},
    utils::get_msg_expiration_from_seconds,
};

const MAX_DISC_PACKET_SIZE: usize = 1280;

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryServerError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("Failed to spawn connection handler")]
    ConnectionError(#[from] ConnectionHandlerError),
    #[error("Failed to decode packet")]
    InvalidPacket(#[from] PacketDecodeErr),
    #[error("Failed to send message")]
    MessageSendFailure(std::io::Error),
    #[error("Only partial message was sent")]
    PartialMessageSent,
}

#[derive(Debug, Clone)]
pub struct DiscoveryServerState {
    local_node: Node,
    local_node_record: Arc<Mutex<NodeRecord>>,
    signer: SigningKey,
    udp_socket: Arc<UdpSocket>,
    kademlia: Kademlia,
}

impl DiscoveryServerState {
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
        }
    }

    async fn handle_listens(&self) -> Result<(), DiscoveryServerError> {
        let mut buf = vec![0; MAX_DISC_PACKET_SIZE];
        loop {
            let (read, _from) = self.udp_socket.recv_from(&mut buf).await?;
            let Ok(packet) = Packet::decode(&buf[..read])
                .inspect_err(|e| warn!(err = ?e, "Failed to decode packet"))
            else {
                continue;
            };
            let mut conn_handle = ConnectionHandler::spawn(self.clone()).await;
            let _ = conn_handle.cast(packet.into()).await;
        }
    }

    async fn ping(&self, node: &Node) -> Result<(), DiscoveryServerError> {
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
            .map_err(DiscoveryServerError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            return Err(DiscoveryServerError::PartialMessageSent);
        }

        debug!(sent = "Ping", to = %format!("{:#x}", node.public_key));

        Ok(())
    }

    async fn pong(&self, ping_hash: H256, node: &Node) -> Result<(), DiscoveryServerError> {
        let mut buf = Vec::new();

        // TODO: Parametrize this expiration.
        let expiration: u64 = get_msg_expiration_from_seconds(20);

        let to = Endpoint {
            ip: node.ip,
            udp_port: node.udp_port,
            tcp_port: node.tcp_port,
        };

        let enr_seq = self.local_node_record.lock().await.seq;

        let pong = Message::Pong(PongMessage::new(to, ping_hash, expiration).with_enr_seq(enr_seq));

        pong.encode_with_header(&mut buf, &self.signer);

        let bytes_sent = self.udp_socket.send_to(&buf, node.udp_addr()).await?;

        if bytes_sent != buf.len() {
            return Err(DiscoveryServerError::PartialMessageSent);
        }

        debug!(sent = "Pong", to = %format!("{:#x}", node.public_key));

        let _ = self.send_enr_request(node).await.inspect_err(
            |e| error!(received = "ENRRequest", to = %format!("{:#x}", node.public_key), err = ?e),
        );

        Ok(())
    }

    async fn send_enr_request(&self, node: &Node) -> Result<(), DiscoveryServerError> {
        let mut buf = Vec::new();

        // TODO: Parametrize this expiration.
        let expiration: u64 = get_msg_expiration_from_seconds(20);

        let enr_req = Message::ENRRequest(ENRRequestMessage::new(expiration));

        enr_req.encode_with_header(&mut buf, &self.signer);

        let bytes_sent = self
            .udp_socket
            .send_to(&buf, node.udp_addr())
            .await
            .map_err(DiscoveryServerError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            return Err(DiscoveryServerError::PartialMessageSent);
        }

        debug!(sent = "ENRRequest", to = %format!("{:#x}", node.public_key));

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum InMessage {
    Listen,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

pub struct DiscoveryServer;

impl DiscoveryServer {
    pub async fn spawn(
        local_node: Node,
        signer: SigningKey,
        udp_socket: Arc<UdpSocket>,
        kademlia: Kademlia,
        bootnodes: Vec<Node>,
    ) -> Result<(), DiscoveryServerError> {
        info!("Starting Discovery Server");

        let local_node_record = Arc::new(Mutex::new(
            NodeRecord::from_node(&local_node, 1, &signer)
                .expect("Failed to create local node record"),
        ));

        let state =
            DiscoveryServerState::new(local_node, local_node_record, signer, udp_socket, kademlia);

        let mut server = DiscoveryServer::start(state.clone());

        let _ = server.cast(InMessage::Listen).await;

        for bootnode in bootnodes {
            let _ = state.ping(&bootnode).await.inspect_err(|e| {
                error!("Failed to ping bootnode: {e}");
            });
        }

        Ok(())
    }
}

impl GenServer for DiscoveryServer {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type State = DiscoveryServerState;
    type Error = DiscoveryServerError;

    fn new() -> Self {
        Self {}
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
        state: Self::State,
    ) -> CastResponse<Self> {
        match message {
            Self::CastMsg::Listen => {
                let _ = state.handle_listens().await.inspect_err(|e| {
                    error!("Failed to handle listens: {e}");
                });
                CastResponse::Stop
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionHandlerError {}

#[derive(Debug, Clone)]
pub enum ConnectionHandlerInMessage {
    Ping {
        message: PingMessage,
        hash: H256,
        sender_public_key: H512,
    },
    Pong(Packet),
    FindNode(Packet),
    Neighbors {
        message: NeighborsMessage,
        sender_public_key: H512,
    },
    ENRResponse {
        message: ENRResponseMessage,
        sender_public_key: H512,
    },
    ENRRequest(Packet),
}

impl From<Packet> for ConnectionHandlerInMessage {
    fn from(packet: Packet) -> Self {
        match packet.get_message() {
            Message::Ping(msg) => Self::Ping {
                message: msg.clone(),
                hash: packet.get_hash(),
                sender_public_key: packet.get_public_key(),
            },
            Message::Pong(..) => Self::Pong(packet),
            Message::FindNode(..) => Self::FindNode(packet),
            Message::Neighbors(msg) => Self::Neighbors {
                message: msg.clone(),
                sender_public_key: packet.get_public_key(),
            },
            Message::ENRResponse(msg) => Self::ENRResponse {
                message: msg.clone(),
                sender_public_key: packet.get_public_key(),
            },
            Message::ENRRequest(..) => Self::ENRRequest(packet),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ConnectionHandlerOutMessage {
    Done,
}

pub struct ConnectionHandler;

impl ConnectionHandler {
    pub async fn spawn(state: DiscoveryServerState) -> GenServerHandle<Self> {
        ConnectionHandler::start(state)
    }
}

impl GenServer for ConnectionHandler {
    type CallMsg = Unused;
    type CastMsg = ConnectionHandlerInMessage;
    type OutMsg = ConnectionHandlerOutMessage;
    type State = DiscoveryServerState;
    type Error = ConnectionHandlerError;

    fn new() -> Self {
        Self {}
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
        state: Self::State,
    ) -> CastResponse<Self> {
        match message {
            Self::CastMsg::Ping {
                message: msg,
                hash,
                sender_public_key,
            } => {
                debug!(received = "Ping", from = %format!("{sender_public_key:#x}"));

                let node = Node::new(
                    msg.from.ip,
                    msg.from.udp_port,
                    msg.from.tcp_port,
                    sender_public_key,
                );

                let _ = state.pong(hash, &node).await.inspect_err(|e| {
                    error!(sent = "Pong", to = %format!("{sender_public_key:#x}"), err = ?e);
                });
            }
            Self::CastMsg::Pong(packet) => {
                debug!(received = "Pong", from = %format!("{:#x}", packet.get_public_key()));
            }
            Self::CastMsg::FindNode(packet) => {
                debug!(received = "FindNode", from = %format!("{:#x}", packet.get_public_key()));
            }
            Self::CastMsg::Neighbors {
                message: msg,
                sender_public_key,
            } => {
                debug!(received = "Neighbors", from = %format!("{sender_public_key:#x}"));

                let mut kademlia = state.kademlia.contacts.lock().await;

                for node in msg.nodes {
                    if let Entry::Vacant(vacant_entry) = kademlia.entry(node.node_id()) {
                        vacant_entry.insert(node);
                        METRICS.record_new_contact().await;
                    };
                }
            }
            Self::CastMsg::ENRRequest(packet) => {
                debug!(received = "ENRRequest", from = %format!("{:#x}", packet.get_public_key()));

                debug!(packet = ?packet);
            }
            Self::CastMsg::ENRResponse {
                message: _msg,
                sender_public_key,
            } => {
                /*
                    - Look up in kademlia the peer associated with this message
                    - Check that the request hash sent matches the one we sent previously (this requires setting it on enrrequest)
                    - Check that the seq number matches the one we have in our table (this requires setting it).
                    - Check valid signature
                    - Take the `eth` part of the record. If it's None, this peer is garbage; if it's set
                */
                debug!(received = "ENRResponse", from = %format!("{sender_public_key:#x}"));
            }
        }
        CastResponse::NoReply(state)
    }
}
