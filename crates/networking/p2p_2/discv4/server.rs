use std::{collections::HashMap, sync::Arc};

use k256::ecdsa::SigningKey;
use keccak_hash::H256;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle},
};
use tokio::{net::UdpSocket, sync::Mutex};
use tracing::{error, info};

use crate::{
    discv4::messages::{
        ENRRequestMessage, ENRResponseMessage, FindNodeMessage, Message, NeighborsMessage, Packet,
        PacketDecodeErr, PingMessage, PongMessage,
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
    local_node_record: Arc<Mutex<NodeRecord>>,
    signer: SigningKey,
    udp_socket: Arc<UdpSocket>,
    kademlia: Arc<Mutex<HashMap<String, String>>>,
}

impl DiscoveryServerState {
    pub fn new(udp_socket: Arc<UdpSocket>, kademlia: Arc<Mutex<HashMap<String, String>>>) -> Self {
        Self {
            udp_socket,
            kademlia,
        }
    }

    async fn handle_listens(&self) -> Result<(), DiscoveryServerError> {
        let mut buf = vec![0; MAX_DISC_PACKET_SIZE];
        loop {
            let (read, from) = self.udp_socket.recv_from(&mut buf).await?;
            info!("Received packet from {from}");
            let packet = Packet::decode(&buf[..read]).expect("Failed to decode packet");
            let mut conn_handle = ConnectionHandler::spawn(self.clone()).await;
            let _ = conn_handle.cast(packet.get_message().clone().into()).await;
        }
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
        addr: String,
        port: u16,
        kademlia: Arc<Mutex<HashMap<String, String>>>,
    ) -> Result<(), DiscoveryServerError> {
        let udp_socket = Arc::new(UdpSocket::bind(format!("{addr}:{port}")).await?);
        let state = DiscoveryServerState::new(udp_socket.clone(), kademlia);
        let mut server = DiscoveryServer::start(state);
        let _ = server.cast(InMessage::Listen).await;
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
    Ping(PingMessage),
    Pong(PongMessage),
    FindNode(FindNodeMessage),
    Neighbors(NeighborsMessage),
    ENRResponse(ENRResponseMessage),
    ENRRequest(ENRRequestMessage),
}

impl From<Message> for ConnectionHandlerInMessage {
    fn from(msg: Message) -> Self {
        match msg {
            Message::Ping(msg) => Self::Ping(msg),
            Message::Pong(msg) => Self::Pong(msg),
            Message::FindNode(msg) => Self::FindNode(msg),
            Message::Neighbors(msg) => Self::Neighbors(msg),
            Message::ENRResponse(msg) => Self::ENRResponse(msg),
            Message::ENRRequest(msg) => Self::ENRRequest(msg),
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
        handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
        state: Self::State,
    ) -> CastResponse<Self> {
        match message {
            Self::CastMsg::Ping(PingMessage {
                version,
                from,
                to,
                expiration,
                enr_seq,
            }) => {
                // Handle Ping message
            }
            Self::CastMsg::Pong(PongMessage {
                to,
                ping_hash,
                expiration,
                enr_seq,
            }) => {
                // Handle Pong message
            }
            Self::CastMsg::FindNode(FindNodeMessage { target, expiration }) => {
                // Handle FindNode message
            }
            Self::CastMsg::Neighbors(NeighborsMessage { nodes, expiration }) => {
                // Handle Neighbors message
            }
            Self::CastMsg::ENRResponse(ENRResponseMessage {
                request_hash,
                node_record,
            }) => {
                // Handle ENRResponse message
            }
            Self::CastMsg::ENRRequest(ENRRequestMessage { expiration }) => {
                // Handle ENRRequest message
            }
        }
        CastResponse::NoReply(state)
    }
}
