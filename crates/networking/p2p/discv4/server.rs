use crate::{
    discv4::{
        codec::Discv4Codec,
        messages::{
            ENRResponseMessage, FindNodeMessage, Message, NeighborsMessage, Packet,
            PacketDecodeErr, PingMessage, PongMessage,
        },
        peer_table::{Contact, OutMessage as PeerTableOutMessage, PeerTable, PeerTableHandle},
    },
    metrics::METRICS,
    types::{Endpoint, Node, NodeRecord},
    utils::{
        get_msg_expiration_from_seconds, is_msg_expired, node_id, public_key_from_signing_key,
        unmap_ipv4in6_address,
    },
};
use ethrex_common::{H512, U256};
use futures::{SinkExt as _, StreamExt, stream::SplitSink};
use keccak_hash::H256;
use rand::rngs::OsRng;
use secp256k1::SecretKey;
use spawned_concurrency::{
    messages::Unused,
    tasks::{
        CastResponse, GenServer, GenServerHandle, InitResult::Success, send_after, send_interval,
        spawn_listener,
    },
};
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{net::UdpSocket, sync::Mutex};
use tokio_util::udp::UdpFramed;
use tracing::{debug, error, info, trace};

pub(crate) const MAX_NODES_IN_NEIGHBORS_PACKET: usize = 16;
const EXPIRATION_SECONDS: u64 = 20;
/// Interval between revalidation checks.
const REVALIDATION_CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60); // 12 hours,
/// Interval between revalidations.
const REVALIDATION_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60); // 12 hours,
/// The initial interval between peer lookups, until the number of peers reaches
/// [target_peers](DiscoverySideCarState::target_peers), or the number of
/// contacts reaches [target_contacts](DiscoverySideCarState::target_contacts).
const INITIAL_LOOKUP_INTERVAL: Duration = Duration::from_secs(5);
const LOOKUP_INTERVAL: Duration = Duration::from_secs(5 * 60); // 5 minutes
const PRUNE_INTERVAL: Duration = Duration::from_secs(5);
/// The target number of RLPx connections to reach.
const TARGET_PEERS: usize = 100;
/// The target number of contacts to maintain in peer_table.
const TARGET_CONTACTS: usize = 100_000;

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryServerError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("Failed to decode packet")]
    InvalidPacket(#[from] PacketDecodeErr),
    #[error("Failed to send message")]
    MessageSendFailure(PacketDecodeErr),
    #[error("Only partial message was sent")]
    PartialMessageSent,
    #[error("Unknown or invalid contact")]
    InvalidContact,
}

#[derive(Debug, Clone)]
pub enum InMessage {
    Message(Box<Discv4Message>),
    Revalidate,
    Lookup,
    Prune,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

type UdpFramedSplitSink =
    SplitSink<UdpFramed<Discv4Codec, Arc<UdpSocket>>, (Message, std::net::SocketAddr)>;

#[derive(Debug)]
pub struct DiscoveryServer {
    local_node: Node,
    local_node_record: Arc<Mutex<NodeRecord>>,
    signer: SecretKey,
    udp_socket: Arc<UdpSocket>,
    /// Sink end of the UdpFramed stream to send messages to peers
    sink: Option<Arc<Mutex<UdpFramedSplitSink>>>,
    peer_table: PeerTableHandle,
}

impl DiscoveryServer {
    pub async fn spawn(
        local_node: Node,
        signer: SecretKey,
        udp_socket: Arc<UdpSocket>,
        mut peer_table: PeerTableHandle,
        bootnodes: Vec<Node>,
    ) -> Result<(), DiscoveryServerError> {
        info!("Starting Discovery Server");

        let local_node_record = Arc::new(Mutex::new(
            NodeRecord::from_node(&local_node, 1, &signer)
                .expect("Failed to create local node record"),
        ));
        let mut discovery_server = Self {
            local_node,
            local_node_record,
            signer,
            udp_socket,
            sink: None,
            peer_table: peer_table.clone(),
        };

        info!(count = bootnodes.len(), "Adding bootnodes");

        for bootnode in &bootnodes {
            discovery_server.send_ping(bootnode).await;
            PeerTable::new_contact(&mut peer_table, bootnode.node_id(), bootnode.clone().into())
                .await;
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
    ) {
        // Ignore packets sent by ourselves
        if node_id(&sender_public_key) == self.local_node.node_id() {
            return;
        }
        match message {
            Message::Ping(ping_message) => {
                trace!(received = "Ping", msg = ?ping_message, from = %format!("{sender_public_key:#x}"));

                if is_msg_expired(ping_message.expiration) {
                    trace!("Ping expired, skipped");
                    return;
                }

                let node = Node::new(
                    unmap_ipv4in6_address(from.ip()),
                    from.port(),
                    ping_message.from.tcp_port,
                    sender_public_key,
                );

                let _ = self.handle_ping(hash, node).await.inspect_err(|e| {
                    error!(sent = "Ping", to = %format!("{sender_public_key:#x}"), err = ?e, "Error handling message");
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
                    trace!("FindNode expired, skipped");
                    return;
                }

                self.handle_find_node(sender_public_key, from).await;
            }
            Message::Neighbors(neighbors_message) => {
                trace!(received = "Neighbors", msg = ?neighbors_message, from = %format!("{sender_public_key:#x}"));

                if is_msg_expired(neighbors_message.expiration) {
                    trace!("Neighbors expired, skipping");
                    return;
                }

                self.handle_neighbors(neighbors_message).await;
            }
            Message::ENRRequest(enrrequest_message) => {
                trace!(received = "ENRRequest", msg = ?enrrequest_message, from = %format!("{sender_public_key:#x}"));

                if is_msg_expired(enrrequest_message.expiration) {
                    trace!("ENRRequest expired, skipping");
                    return;
                }

                self.handle_enr_request(sender_public_key, from, hash).await;
            }
            Message::ENRResponse(enrresponse_message) => {
                /*
                    TODO
                    https://github.com/lambdaclass/ethrex/issues/4412
                    - Look up in peer_table the peer associated with this message
                    - Check that the request hash sent matches the one we sent previously (this requires setting it on enrrequest)
                    - Check that the seq number matches the one we have in our table (this requires setting it).
                    - Check valid signature
                    - Take the `eth` part of the record. If it's None, this peer is garbage; if it's set
                */
                trace!(received = "ENRResponse", msg = ?enrresponse_message, from = %format!("{sender_public_key:#x}"));
            }
        }
    }

    async fn revalidate(&mut self) {
        for contact in
            PeerTable::get_contacts_to_revalidate(&mut self.peer_table, REVALIDATION_INTERVAL)
                .await
                // TODO proper error handling
                .unwrap_or(Vec::new())
        {
            self.send_ping(&contact.node).await;
        }
    }

    async fn lookup(&mut self) {
        for contact in PeerTable::get_contacts_for_lookup(&mut self.peer_table, 20)
            .await
            // TODO proper error handling
            .unwrap_or(Vec::new())
        {
            if let Err(err) = self.send_find_node(&contact.node).await {
                error!(sent = "FindNode", to = %format!("{:#x}", contact.node.public_key), err = ?err, "Error sending message");
                PeerTable::set_disposable(&mut self.peer_table, &contact.node.node_id()).await;
                //contact.disposable = true;
                METRICS.record_new_discarded_node().await;
            }

            PeerTable::increment_find_node_sent(&mut self.peer_table, &contact.node.node_id())
                .await;
            //contact.n_find_node_sent += 1;
        }
    }

    async fn prune(&mut self) {
        PeerTable::prune(&mut self.peer_table)
            .await
            .inspect_err(|e| error!(err= ?e, "Failed to prune peer table"));
    }

    async fn get_lookup_interval(&mut self) -> Duration {
        if PeerTable::target_reached(&mut self.peer_table, TARGET_CONTACTS, TARGET_PEERS)
            .await
            .unwrap_or(false)
        {
            INITIAL_LOOKUP_INTERVAL
        } else {
            trace!("Reached target number of peers or contacts. Using longer lookup interval.");
            LOOKUP_INTERVAL
        }
    }

    async fn send_ping(&mut self, node: &Node) {
        match self.send_ping_internal(node).await {
            Ok(ping_hash) => {
                METRICS.record_ping_sent().await;
                PeerTable::record_ping_sent(&mut self.peer_table, &node.node_id(), ping_hash).await;
            }
            Err(err) => {
                error!(sent = "Ping", to = %format!("{:#x}", node.public_key), err = ?err, "Error sending message");
                PeerTable::set_disposable(&mut self.peer_table, &node.node_id()).await;
                METRICS.record_new_discarded_node().await;
            }
        }
    }

    async fn send_ping_internal(&self, node: &Node) -> Result<H256, DiscoveryServerError> {
        let mut buf = Vec::new();
        // TODO: Parametrize this expiration.
        let expiration: u64 = get_msg_expiration_from_seconds(EXPIRATION_SECONDS);
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
        // We do not use the Sink/Codec here, as we already encoded the message to calculate hash.
        self.udp_socket.send_to(&buf, node.udp_addr()).await?;
        debug!(sent = "Ping", to = %format!("{:#x}", node.public_key));
        Ok(H256::from(ping_hash))
    }

    async fn send_pong(&self, ping_hash: H256, node: &Node) -> Result<(), DiscoveryServerError> {
        // TODO: Parametrize this expiration.
        let expiration: u64 = get_msg_expiration_from_seconds(EXPIRATION_SECONDS);

        let to = Endpoint {
            ip: node.ip,
            udp_port: node.udp_port,
            tcp_port: node.tcp_port,
        };

        let enr_seq = self.local_node_record.lock().await.seq;

        let pong = Message::Pong(PongMessage::new(to, ping_hash, expiration).with_enr_seq(enr_seq));

        self.send(pong, node.udp_addr()).await?;

        debug!(sent = "Pong", to = %format!("{:#x}", node.public_key));

        Ok(())
    }

    async fn send_find_node(&self, node: &Node) -> Result<(), DiscoveryServerError> {
        let expiration: u64 = get_msg_expiration_from_seconds(EXPIRATION_SECONDS);

        let random_priv_key = SecretKey::new(&mut OsRng);
        let random_pub_key = public_key_from_signing_key(&random_priv_key);

        let msg = Message::FindNode(FindNodeMessage::new(random_pub_key, expiration));
        self.send(msg, node.udp_addr()).await?;

        debug!(sent = "FindNode", to = %format!("{:#x}", node.public_key));

        Ok(())
    }

    async fn send_neighbors(
        &self,
        neighbors: Vec<Node>,
        node: &Node,
    ) -> Result<(), DiscoveryServerError> {
        // TODO: Parametrize this expiration.
        let expiration: u64 = get_msg_expiration_from_seconds(EXPIRATION_SECONDS);

        let msg = Message::Neighbors(NeighborsMessage::new(neighbors, expiration));

        self.send(msg, node.udp_addr()).await?;

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

        self.send(msg, from).await?;

        Ok(())
    }

    async fn handle_ping(&mut self, hash: H256, node: Node) -> Result<(), DiscoveryServerError> {
        self.send_pong(hash, &node).await?;

        if PeerTable::insert_if_new(&mut self.peer_table, &node)
            .await
            .unwrap_or(false)
        {
            self.send_ping(&node).await;
        }

        Ok(())
    }

    async fn handle_pong(&mut self, message: PongMessage, node_id: H256) {
        PeerTable::record_pong_received(&mut self.peer_table, &node_id, message.ping_hash).await;
    }

    async fn handle_find_node(&mut self, sender_public_key: H512, from: SocketAddr) {
        let node_id = node_id(&sender_public_key);
        if let Ok(contact) = self
            .validate_contact(sender_public_key, node_id, from, "FindNode")
            .await
        {
            let neighbors = PeerTable::get_closest_nodes(&mut self.peer_table, &node_id)
                .await
                // TODO: Proper error handling
                .unwrap_or(Vec::new());

            // we are sending the neighbors in 2 different messages to avoid exceeding the
            // maximum packet size
            for chunk in neighbors.chunks(8) {
                let _ = self
                    .send_neighbors(chunk.to_vec(), &contact.node)
                    .await
                    .inspect_err(|e| {
                        error!(sent = "Neighbors", to = %format!("{sender_public_key:#x}"), err = ?e, "Error sending message");
                    });
            }
        }
    }

    async fn handle_neighbors(&mut self, neighbors_message: NeighborsMessage) {
        // TODO(#3746): check that we requested neighbors from the node
        PeerTable::new_contacts(
            &mut self.peer_table,
            neighbors_message.nodes,
            self.local_node.node_id(),
        )
        .await;
    }

    async fn handle_enr_request(&mut self, sender_public_key: H512, from: SocketAddr, hash: H256) {
        let node_id = node_id(&sender_public_key);

        if self
            .validate_contact(sender_public_key, node_id, from, "ENRRequest")
            .await.is_err()
        {
            return;
        }

        if let Err(err) = self.send_enr_response(hash, from).await {
            error!(sent = "ENRResponse", to = %format!("{from}"), err = ?err, "Error sending message");
            return;
        }

        PeerTable::knows_us(&mut self.peer_table, &node_id).await;
    }

    async fn validate_contact(
        &mut self,
        sender_public_key: H512,
        node_id: H256,
        from: SocketAddr,
        message_type: &str,
    ) -> Result<Contact, DiscoveryServerError> {
        match PeerTable::validate_contact(&mut self.peer_table, &node_id, from.ip())
            .await
            // TODO proper error handling
            .unwrap_or(PeerTableOutMessage::UnknownContact)
        {
            PeerTableOutMessage::UnknownContact => {
                debug!(received = message_type, to = %format!("{sender_public_key:#x}"), "Unknown contact, skipping");
                Err(DiscoveryServerError::InvalidContact)
            }
            PeerTableOutMessage::InvalidContact => {
                debug!(received = message_type, to = %format!("{sender_public_key:#x}"), "Contact not validated, skipping");
                Err(DiscoveryServerError::InvalidContact)
            }
            // Check that the IP address from which we receive the request matches the one we have stored to prevent amplification attacks
            // This prevents an attack vector where the discovery protocol could be used to amplify traffic in a DDOS attack.
            // A malicious actor would send a findnode request with the IP address and UDP port of the target as the source address.
            // The recipient of the findnode packet would then send a neighbors packet (which is a much bigger packet than findnode) to the victim.
            PeerTableOutMessage::IpMismatch => {
                debug!(received = message_type, to = %format!("{sender_public_key:#x}"), "IP address mismatch, skipping");
                Err(DiscoveryServerError::InvalidContact)
            }
            PeerTableOutMessage::ValidContact(contact) => Ok(contact),
            _ => unreachable!(),
        }
    }

    async fn send(&self, message: Message, addr: SocketAddr) -> Result<(), DiscoveryServerError> {
        if let Some(s) = &self.sink {
            s.lock()
                .await
                .send((message.clone(), addr))
                .await
                // Logging extra info to solve https://github.com/lambdaclass/ethrex/issues/4492
                // Remove next line once the cause of the error is found
                .inspect_err(
                    |e| error!(sending = ?message, addr = ?addr, err=?e, "Error sending message"),
                )
                .map_err(DiscoveryServerError::MessageSendFailure)
        } else {
            error!("Trying to send a message through a non-initialized UdpSocket");
            Ok(())
        }
    }
}

impl GenServer for DiscoveryServer {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = DiscoveryServerError;

    async fn init(
        mut self,
        handle: &GenServerHandle<Self>,
    ) -> Result<spawned_concurrency::tasks::InitResult<Self>, Self::Error> {
        let framed = UdpFramed::new(self.udp_socket.clone(), Discv4Codec::new(self.signer));
        let (sink, stream) = framed.split();
        self.sink = Some(Arc::new(Mutex::new(sink)));

        spawn_listener(
            handle.clone(),
            stream.filter_map(|result| async move {
                match result {
                    Ok((msg, addr)) => {
                        Some(InMessage::Message(Box::new(Discv4Message::from(msg, addr))))
                    }
                    Err(e) => {
                        debug!(error=?e, "Error receiving Discv4 message");
                        // Skipping invalid data
                        None
                    }
                }
            }),
        );
        send_interval(
            REVALIDATION_CHECK_INTERVAL,
            handle.clone(),
            InMessage::Revalidate,
        );
        send_interval(PRUNE_INTERVAL, handle.clone(), InMessage::Prune);
        let _ = handle.clone().cast(InMessage::Lookup).await;

        Ok(Success(self))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            Self::CastMsg::Message(message) => {
                self.handle_message(*message).await;
            }
            Self::CastMsg::Revalidate => {
                trace!(received = "Revalidate");
                self.revalidate().await;
            }
            Self::CastMsg::Lookup => {
                trace!(received = "Lookup");
                self.lookup().await;

                let interval = self.get_lookup_interval().await;
                send_after(interval, handle.clone(), Self::CastMsg::Lookup);
            }
            Self::CastMsg::Prune => {
                trace!(received = "Prune");
                self.prune().await;
            }
        }
        CastResponse::NoReply
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

// TODO: Reimplement tests removed during snap sync refactor
//       https://github.com/lambdaclass/ethrex/issues/4423
