use std::{
    collections::{BTreeMap, btree_map::Entry},
    net::SocketAddr,
    sync::Arc,
};

use ethrex_common::{H512, U256};
use keccak_hash::H256;
use secp256k1::SecretKey;
use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, InitResult::Success, spawn_listener},
};
use tokio::{net::UdpSocket, sync::Mutex};
use tokio_util::udp::UdpFramed;

use tracing::{debug, error, info, trace};

use crate::{
    discv4::codec::Discv4Codec,
    utils::{is_msg_expired, unmap_ipv4in6_address},
};
use crate::{
    discv4::messages::{
        ENRResponseMessage, Message, NeighborsMessage, Packet, PacketDecodeErr, PingMessage,
        PongMessage,
    },
    kademlia::{Contact, Kademlia},
    metrics::METRICS,
    types::{Endpoint, Node, NodeRecord},
    utils::{get_msg_expiration_from_seconds, node_id},
};

const MAX_NODES_IN_NEIGHBORS_PACKET: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryServerError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("Failed to decode packet")]
    InvalidPacket(#[from] PacketDecodeErr),
    #[error("Failed to send message")]
    MessageSendFailure(std::io::Error),
    #[error("Only partial message was sent")]
    PartialMessageSent,
}

#[derive(Debug, Clone)]
pub enum InMessage {
    Message(Discv4Message),
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

#[derive(Debug)]
pub struct DiscoveryServer {
    local_node: Node,
    local_node_record: Arc<Mutex<NodeRecord>>,
    signer: SecretKey,
    udp_socket: Arc<UdpSocket>,
    kademlia: Kademlia,
}

impl DiscoveryServer {
    pub async fn spawn(
        local_node: Node,
        signer: SecretKey,
        udp_socket: Arc<UdpSocket>,
        kademlia: Kademlia,
        bootnodes: Vec<Node>,
    ) -> Result<(), DiscoveryServerError> {
        info!("Starting Discovery Server");

        let local_node_record = Arc::new(Mutex::new(
            NodeRecord::from_node(&local_node, 1, &signer)
                .expect("Failed to create local node record"),
        ));
        let discovery_server = Self {
            local_node,
            local_node_record,
            signer,
            udp_socket,
            kademlia: kademlia.clone(),
        };

        info!("Pinging {} bootnodes", bootnodes.len());

        let mut table = kademlia.table.lock().await;

        for bootnode in &bootnodes {
            let _ = discovery_server
                .send_ping(&bootnode)
                .await
                .inspect_err(|e| {
                    error!("Failed to ping bootnode: {e}");
                });

            table.insert(bootnode.node_id(), bootnode.clone().into());
        }

        discovery_server.start();

        Ok(())
    }

    async fn handle_message(
        &mut self,
        Discv4Message {
            from,
            message,
            hash,
            sender_public_key,
        }: Discv4Message,
    ) -> CastResponse {
        // Ignore packets sent by ourselves
        if node_id(&sender_public_key) != self.local_node.node_id() {
            match message {
                Message::Ping(ping_message) => {
                    trace!(received = "Ping", msg = ?ping_message, from = %format!("{sender_public_key:#x}"));

                    if is_msg_expired(ping_message.expiration) {
                        trace!("Ping expired");
                        return CastResponse::Stop;
                    }

                    let sender_ip = unmap_ipv4in6_address(from.ip());
                    let node = Node::new(
                        sender_ip,
                        from.port(),
                        ping_message.from.tcp_port,
                        sender_public_key,
                    );

                    let _ = self.handle_ping(hash, node).await.inspect_err(|e| {
                        error!(sent = "Ping", to = %format!("{sender_public_key:#x}"), err = ?e);
                    });
                }
                Message::Pong(pong_message) => {
                    trace!(received = "Pong", msg = ?pong_message, from = %format!("{:#x}", sender_public_key));

                    let node_id = node_id(&sender_public_key);

                    self.handle_pong(pong_message, node_id).await;
                }
                Message::FindNode(find_node_message) => {
                    trace!(received = "FindNode", msg = ?find_node_message, from = %format!("{:#x}", sender_public_key));

                    if is_msg_expired(find_node_message.expiration) {
                        trace!("FindNode expired");
                        return CastResponse::Stop;
                    }
                    let node_id = node_id(&sender_public_key);

                    let table = self.kademlia.table.lock().await;

                    let Some(contact) = table.get(&node_id) else {
                        return CastResponse::Stop;
                    };
                    if !contact.was_validated() {
                        debug!(received = "FindNode", to = %format!("{sender_public_key:#x}"), "Contact not validated, skipping");
                        return CastResponse::Stop;
                    }
                    let node = contact.node.clone();

                    // Check that the IP address from which we receive the request matches the one we have stored to prevent amplification attacks
                    // This prevents an attack vector where the discovery protocol could be used to amplify traffic in a DDOS attack.
                    // A malicious actor would send a findnode request with the IP address and UDP port of the target as the source address.
                    // The recipient of the findnode packet would then send a neighbors packet (which is a much bigger packet than findnode) to the victim.
                    if from.ip() != node.ip {
                        debug!(received = "FindNode", to = %format!("{sender_public_key:#x}"), "IP address mismatch, skipping");
                        return CastResponse::Stop;
                    }

                    let neighbors = get_closest_nodes(node_id, table.clone());

                    drop(table);

                    // we are sending the neighbors in 2 different messages to avoid exceeding the
                    // maximum packet size
                    for chunk in neighbors.chunks(8) {
                        let _ = self.send_neighbors(chunk.to_vec(), &node).await.inspect_err(|e| {
                            error!(sent = "Neighbors", to = %format!("{sender_public_key:#x}"), err = ?e);
                        });
                    }
                }
                Message::Neighbors(neighbors_message) => {
                    trace!(received = "Neighbors", msg = ?neighbors_message, from = %format!("{sender_public_key:#x}"));

                    if is_msg_expired(neighbors_message.expiration) {
                        trace!("Neighbors expired");
                        return CastResponse::Stop;
                    }

                    // TODO(#3746): check that we requested neighbors from the node

                    let mut contacts = self.kademlia.table.lock().await;
                    let discarded_contacts = self.kademlia.discarded_contacts.lock().await;

                    for node in neighbors_message.nodes {
                        let node_id = node.node_id();
                        if let Entry::Vacant(vacant_entry) = contacts.entry(node_id) {
                            if !discarded_contacts.contains(&node_id)
                                && node_id != self.local_node.node_id()
                            {
                                vacant_entry.insert(Contact::from(node));
                                METRICS.record_new_discovery().await;
                            }
                        };
                    }
                }
                Message::ENRRequest(enrrequest_message) => {
                    trace!(received = "ENRRequest", msg = ?enrrequest_message, from = %format!("{sender_public_key:#x}"));

                    if is_msg_expired(enrrequest_message.expiration) {
                        trace!("ENRRequest expired");
                        return CastResponse::Stop;
                    }
                    let node_id = node_id(&sender_public_key);

                    let mut table = self.kademlia.table.lock().await;

                    let Some(contact) = table.get(&node_id) else {
                        return CastResponse::Stop;
                    };
                    if !contact.was_validated() {
                        debug!(received = "ENRRequest", to = %format!("{sender_public_key:#x}"), "Contact not validated, skipping");
                        return CastResponse::Stop;
                    }

                    if let Err(err) = self.send_enr_response(hash, from).await {
                        error!(sent = "ENRResponse", to = %format!("{from}"), err = ?err);
                        return CastResponse::Stop;
                    }

                    table.entry(node_id).and_modify(|c| c.knows_us = true);
                }
                Message::ENRResponse(enrresponse_message) => {
                    /*
                        - Look up in kademlia the peer associated with this message
                        - Check that the request hash sent matches the one we sent previously (this requires setting it on enrrequest)
                        - Check that the seq number matches the one we have in our table (this requires setting it).
                        - Check valid signature
                        - Take the `eth` part of the record. If it's None, this peer is garbage; if it's set
                    */
                    trace!(received = "ENRResponse", msg = ?enrresponse_message, from = %format!("{sender_public_key:#x}"));
                }
            }
        }
        CastResponse::NoReply
    }

    async fn send_ping(&self, node: &Node) -> Result<H256, DiscoveryServerError> {
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

    async fn send_pong(&self, ping_hash: H256, node: &Node) -> Result<(), DiscoveryServerError> {
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

    async fn handle_ping(&self, hash: H256, node: Node) -> Result<(), DiscoveryServerError> {
        self.send_pong(hash, &node).await?;

        let mut table = self.kademlia.table.lock().await;

        match table.entry(node.node_id()) {
            Entry::Occupied(_) => (),
            Entry::Vacant(entry) => {
                let ping_hash = self.send_ping(&node).await?;
                let contact = entry.insert(Contact::from(node));
                contact.record_sent_ping(ping_hash);
            }
        }

        Ok(())
    }

    async fn handle_pong(&self, message: PongMessage, node_id: H256) {
        let mut contacts = self.kademlia.table.lock().await;

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
    }
}

impl GenServer for DiscoveryServer {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = DiscoveryServerError;

    async fn init(
        self,
        handle: &GenServerHandle<Self>,
    ) -> Result<spawned_concurrency::tasks::InitResult<Self>, Self::Error> {
        let stream = UdpFramed::new(self.udp_socket.clone(), Discv4Codec::new());

        spawn_listener(
            handle.clone(),
            |(msg, addr)| InMessage::Message(Discv4Message::from(msg, addr)),
            stream,
        );

        Ok(Success(self))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        _handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            Self::CastMsg::Message(message) => self.handle_message(message).await,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Discv4Message {
    from: SocketAddr,
    message: Message,
    hash: H256,
    sender_public_key: H512,
}

impl Discv4Message {
    pub fn from(packet: Packet, from: SocketAddr) -> Self {
        Self {
            from,
            message: packet.get_message().clone(),
            hash: packet.get_hash(),
            sender_public_key: packet.get_public_key(),
        }
    }

    pub fn get_node_id(&self) -> H256 {
        node_id(&self.sender_public_key)
    }
}

#[derive(Debug, Clone)]
pub enum ConnectionHandlerOutMessage {
    Done,
}

// TODO: SNAP SYNC: REIMPL TESTS

/// Returns the nodes closest to the given `node_id`.
pub fn get_closest_nodes(node_id: H256, table: BTreeMap<H256, Contact>) -> Vec<Node> {
    let mut nodes: Vec<(Node, usize)> = vec![];

    for (contact_id, contact) in &table {
        let distance = distance(&node_id, contact_id);
        if nodes.len() < MAX_NODES_IN_NEIGHBORS_PACKET {
            nodes.push((contact.node.clone(), distance));
        } else {
            for (i, (_, dis)) in &mut nodes.iter().enumerate() {
                if distance < *dis {
                    nodes[i] = (contact.node.clone(), distance);
                    break;
                }
            }
        }
    }
    nodes.into_iter().map(|(node, _distance)| node).collect()
}

pub fn distance(node_id_1: &H256, node_id_2: &H256) -> usize {
    let xor = node_id_1 ^ node_id_2;
    let distance = U256::from_big_endian(xor.as_bytes());
    distance.bits().saturating_sub(1)
}
