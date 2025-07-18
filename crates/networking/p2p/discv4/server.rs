use std::{collections::btree_map::Entry, net::SocketAddr, sync::Arc};

use ethrex_common::{H512, types::ForkId};
use k256::ecdsa::SigningKey;
use keccak_hash::H256;
use rand::{rngs::OsRng, seq::IteratorRandom};
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle},
};
use tokio::{net::UdpSocket, sync::Mutex};
use tracing::{debug, error, info, trace, warn};

use crate::utils::{is_expired, unmap_ipv4in6_address};
use crate::{
    discv4::messages::{
        ENRRequestMessage, ENRResponseMessage, FindNodeMessage, Message, NeighborsMessage, Packet,
        PacketDecodeErr, PingMessage, PongMessage,
    },
    kademlia::{Contact, Kademlia},
    metrics::METRICS,
    types::{Endpoint, Node, NodeRecord},
    utils::{get_msg_expiration_from_seconds, node_id},
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
            let (read, from) = self.udp_socket.recv_from(&mut buf).await?;
            let Ok(packet) = Packet::decode(&buf[..read])
                .inspect_err(|e| warn!(err = ?e, "Failed to decode packet"))
            else {
                continue;
            };
            let mut conn_handle = ConnectionHandler::spawn(self.clone()).await;
            let _ = conn_handle
                .cast(ConnectionHandlerInMessage::from(packet, from))
                .await;
        }
    }

    async fn ping(&self, node: &Node) -> Result<H256, DiscoveryServerError> {
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

        let ping_hash: [u8; 32] = buf[..32]
            .try_into()
            .expect("first 32 bytes are the message hash");

        let bytes_sent = self
            .udp_socket
            .send_to(&buf, node.udp_addr())
            .await
            .map_err(DiscoveryServerError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            return Err(DiscoveryServerError::PartialMessageSent);
        }

        debug!(sent = "Ping", to = %format!("{:#x}", node.public_key));

        Ok(H256::from(ping_hash))
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

        Ok(())
    }

    async fn send_neighbors(
        &self,
        neighbors: Vec<Node>,
        node: &Node,
    ) -> Result<(), DiscoveryServerError> {
        let mut buf = Vec::new();

        // TODO: Parametrize this expiration.
        let expiration: u64 = get_msg_expiration_from_seconds(20);

        let msg = Message::Neighbors(NeighborsMessage::new(neighbors, expiration));

        msg.encode_with_header(&mut buf, &self.signer);

        let bytes_sent = self.udp_socket.send_to(&buf, node.udp_addr()).await?;

        if bytes_sent != buf.len() {
            return Err(DiscoveryServerError::PartialMessageSent);
        }

        debug!(sent = "Neighbors", to = %format!("{:#x}", node.public_key));

        Ok(())
    }

    async fn send_enr_response(
        &self,
        request_hash: H256,
        from: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        let node_record = self.local_node_record.lock().await;

        let msg = Message::ENRResponse(ENRResponseMessage::new(request_hash, node_record.clone()));

        let mut buf = vec![];

        msg.encode_with_header(&mut buf, &self.signer);

        let bytes_sent = self
            .udp_socket
            .send_to(&buf, from)
            .await
            .map_err(DiscoveryServerError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            return Err(DiscoveryServerError::PartialMessageSent);
        }

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

    async fn send_find_node(&self, node: &Node) -> Result<(), DiscoveryServerError> {
        let expiration: u64 = get_msg_expiration_from_seconds(20);

        let msg = Message::FindNode(FindNodeMessage::new(node.public_key, expiration));

        let mut buf = Vec::new();
        msg.encode_with_header(&mut buf, &self.signer);
        let bytes_sent = self
            .udp_socket
            .send_to(&buf, SocketAddr::new(node.ip, node.udp_port))
            .await
            .map_err(DiscoveryServerError::MessageSendFailure)?;

        if bytes_sent != buf.len() {
            return Err(DiscoveryServerError::PartialMessageSent);
        }

        debug!(sent = "FindNode", to = %format!("{:#x}", node.public_key));

        Ok(())
    }

    async fn handle_ping(&self, hash: H256, node: Node) -> Result<(), DiscoveryServerError> {
        self.pong(hash, &node).await?;

        let mut table = self.kademlia.table.lock().await;

        match table.entry(node.node_id()) {
            Entry::Occupied(_) => (),
            Entry::Vacant(entry) => {
                let ping_hash = self.ping(&node).await?;
                let contact = entry.insert(Contact::from(node));
                contact.record_sent_ping(ping_hash);
            }
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
        local_node: Node,
        signer: SigningKey,
        fork_id: &ForkId,
        udp_socket: Arc<UdpSocket>,
        kademlia: Kademlia,
        bootnodes: Vec<Node>,
    ) -> Result<(), DiscoveryServerError> {
        info!("Starting Discovery Server");

        let local_node_record = Arc::new(Mutex::new(
            NodeRecord::from_node(&local_node, 1, &signer, fork_id.clone())
                .expect("Failed to create local node record"),
        ));

        let state = DiscoveryServerState::new(
            local_node,
            local_node_record,
            signer,
            udp_socket,
            kademlia.clone(),
        );

        let mut server = DiscoveryServer::start(state.clone());

        let _ = server.cast(InMessage::Listen).await;

        info!("Pinging {} bootnodes", bootnodes.len());

        for bootnode in bootnodes {
            let _ = state.ping(&bootnode).await.inspect_err(|e| {
                error!("Failed to ping bootnode: {e}");
            });

            kademlia
                .table
                .lock()
                .await
                .insert(bootnode.node_id(), bootnode.into());
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
        from: SocketAddr,
        message: PingMessage,
        hash: H256,
        sender_public_key: H512,
    },
    Pong {
        message: PongMessage,
        sender_public_key: H512,
    },
    FindNode {
        message: FindNodeMessage,
        sender_public_key: H512,
    },
    Neighbors {
        message: NeighborsMessage,
        sender_public_key: H512,
    },
    ENRResponse {
        message: ENRResponseMessage,
        sender_public_key: H512,
    },
    ENRRequest {
        message: ENRRequestMessage,
        from: SocketAddr,
        hash: H256,
        sender_public_key: H512,
    },
}

impl ConnectionHandlerInMessage {
    pub fn from(packet: Packet, from: SocketAddr) -> Self {
        match packet.get_message() {
            Message::Ping(msg) => Self::Ping {
                from,
                message: msg.clone(),
                hash: packet.get_hash(),
                sender_public_key: packet.get_public_key(),
            },
            Message::Pong(msg) => Self::Pong {
                message: *msg,
                sender_public_key: packet.get_public_key(),
            },
            Message::FindNode(msg) => Self::FindNode {
                message: msg.clone(),
                sender_public_key: packet.get_public_key(),
            },
            Message::Neighbors(msg) => Self::Neighbors {
                message: msg.clone(),
                sender_public_key: packet.get_public_key(),
            },
            Message::ENRResponse(msg) => Self::ENRResponse {
                message: msg.clone(),
                sender_public_key: packet.get_public_key(),
            },
            Message::ENRRequest(msg) => Self::ENRRequest {
                message: *msg,
                from,
                hash: packet.get_hash(),
                sender_public_key: packet.get_public_key(),
            },
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
                from,
                message: msg,
                hash,
                sender_public_key,
            } => {
                trace!(received = "Ping", msg = ?msg, from = %format!("{sender_public_key:#x}"));

                if is_expired(msg.expiration) {
                    trace!("Ping expired");
                    return CastResponse::Stop;
                }

                let sender_ip = unmap_ipv4in6_address(from.ip());
                let node = Node::new(sender_ip, from.port(), msg.from.tcp_port, sender_public_key);

                let _ = state.handle_ping(hash, node).await.inspect_err(|e| {
                    error!(sent = "Ping", to = %format!("{sender_public_key:#x}"), err = ?e);
                });
            }
            Self::CastMsg::Pong {
                message,
                sender_public_key,
            } => {
                trace!(received = "Pong", msg = ?message, from = %format!("{:#x}", sender_public_key));

                let node_id = node_id(&sender_public_key);

                handle_pong(&state, message, node_id).await;
            }
            Self::CastMsg::FindNode {
                message,
                sender_public_key,
            } => {
                trace!(received = "FindNode", msg = ?message, from = %format!("{:#x}", sender_public_key));

                let node_id = node_id(&sender_public_key);

                let table = state.kademlia.table.lock().await;

                let Some(contact) = table.get(&node_id) else {
                    drop(table);
                    return CastResponse::Stop;
                };

                let neighbors = table
                    .iter()
                    .map(|(_, c)| c.node.clone())
                    .choose_multiple(&mut OsRng, 16);

                let _ = state.send_neighbors(neighbors, &contact.node).await.inspect_err(|e| {
                    error!(sent = "Neighbors", to = %format!("{sender_public_key:#x}"), err = ?e);
                });
            }
            Self::CastMsg::Neighbors {
                message: msg,
                sender_public_key,
            } => {
                trace!(received = "Neighbors", msg = ?msg, from = %format!("{sender_public_key:#x}"));

                if is_expired(msg.expiration) {
                    trace!("Neighbors expired");
                    return CastResponse::Stop;
                }

                let mut contacts = state.kademlia.table.lock().await;
                let discarded_contacts = state.kademlia.discarded_contacts.lock().await;

                for node in msg.nodes {
                    let node_id = node.node_id();
                    if let Entry::Vacant(vacant_entry) = contacts.entry(node_id) {
                        if !discarded_contacts.contains(&node_id) {
                            vacant_entry.insert(Contact::from(node));
                            METRICS.record_new_discovery().await;
                        }
                    };
                }
            }
            Self::CastMsg::ENRRequest {
                message: msg,
                from,
                hash,
                sender_public_key,
            } => {
                trace!(received = "ENRRequest", msg = ?msg, from = %format!("{sender_public_key:#x}"));

                if is_expired(msg.expiration) {
                    trace!("ENRRequest expired");
                    return CastResponse::Stop;
                }

                if let Err(err) = state.send_enr_response(hash, from).await {
                    error!(sent = "ENRResponse", to = %format!("{from}"), err = ?err);
                    return CastResponse::Stop;
                }

                state
                    .kademlia
                    .table
                    .lock()
                    .await
                    .entry(node_id(&sender_public_key))
                    .and_modify(|c| c.knows_us = true);
            }
            Self::CastMsg::ENRResponse {
                message: msg,
                sender_public_key,
            } => {
                /*
                    - Look up in kademlia the peer associated with this message
                    - Check that the request hash sent matches the one we sent previously (this requires setting it on enrrequest)
                    - Check that the seq number matches the one we have in our table (this requires setting it).
                    - Check valid signature
                    - Take the `eth` part of the record. If it's None, this peer is garbage; if it's set
                */
                trace!(received = "ENRResponse", msg = ?msg, from = %format!("{sender_public_key:#x}"));
            }
        }
        CastResponse::Stop
    }
}

async fn handle_pong(state: &DiscoveryServerState, message: PongMessage, node_id: H256) {
    let mut contacts = state.kademlia.table.lock().await;

    // Received a pong from a node we don't know about
    let Some(contact) = contacts.get_mut(&node_id) else {
        return;
    };
    // Received a pong for an unknown ping
    if !contact
        .ping_hash
        .map(|ph| ph == message.ping_hash)
        .unwrap_or(false)
    {
        return;
    }
    contact.ping_hash = None;

    let node = contact.node.clone();

    let _ = state.send_enr_request(&node).await.inspect_err(
        |e| error!(received = "ENRRequest", to = %format!("{:#x}", node.public_key), err = ?e),
    );

    let _ = state.send_find_node(&node).await.inspect_err(
        |e| error!(sent = "FindNode", to = %format!("{:#x}", node.public_key), err = ?e),
    );
}
