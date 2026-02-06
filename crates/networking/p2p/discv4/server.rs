use crate::{
    backend,
    discv4::{
        codec::Discv4Codec,
        messages::{
            ENRRequestMessage, ENRResponseMessage, FindNodeMessage, Message, NeighborsMessage,
            Packet, PacketDecodeErr, PingMessage, PongMessage,
        },
        peer_table::{Contact, OutMessage as PeerTableOutMessage, PeerTable, PeerTableError},
    },
    metrics::METRICS,
    types::{Endpoint, Node, NodeRecord},
    utils::{
        get_msg_expiration_from_seconds, is_msg_expired, node_id, public_key_from_signing_key,
    },
};
use bytes::BytesMut;
use ethrex_common::{H256, H512, types::ForkId};
use ethrex_storage::{Store, error::StoreError};
use futures::StreamExt;
use rand::rngs::OsRng;
use secp256k1::SecretKey;
use spawned_concurrency::{
    messages::Unused,
    tasks::{
        CastResponse, GenServer, GenServerHandle, InitResult::Success, send_after, send_interval,
        send_message_on, spawn_listener,
    },
};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::UdpSocket;
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
pub const INITIAL_LOOKUP_INTERVAL_MS: f64 = 500.0;
pub const LOOKUP_INTERVAL_MS: f64 = 5000.0;
/// Slow maintenance interval when target peers are met (30 seconds).
const MAINTENANCE_LOOKUP_INTERVAL: Duration = Duration::from_secs(30);
const CHANGE_FIND_NODE_MESSAGE_INTERVAL: Duration = Duration::from_secs(5);
const PRUNE_INTERVAL: Duration = Duration::from_secs(5);

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
    #[error(transparent)]
    PeerTable(#[from] PeerTableError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, Clone)]
pub enum InMessage {
    Message(Box<Discv4Message>),
    Revalidate,
    Lookup,
    Prune,
    ChangeFindNodeMessage,
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

#[derive(Debug)]
pub struct DiscoveryServer {
    local_node: Node,
    local_node_record: NodeRecord,
    signer: SecretKey,
    udp_socket: Arc<UdpSocket>,
    store: Store,
    peer_table: PeerTable,
    /// The last `FindNode` message sent, cached due to message
    /// signatures being expensive.
    find_node_message: BytesMut,
    initial_lookup_interval: f64,
    /// Alternates between regular lookup and ENR lookup on each tick.
    next_lookup_is_enr: bool,
}

impl DiscoveryServer {
    pub async fn spawn(
        storage: Store,
        local_node: Node,
        signer: SecretKey,
        udp_socket: UdpSocket,
        mut peer_table: PeerTable,
        bootnodes: Vec<Node>,
        initial_lookup_interval: f64,
    ) -> Result<(), DiscoveryServerError> {
        info!("Starting Discovery Server");

        let mut local_node_record = NodeRecord::from_node(&local_node, 1, &signer)
            .expect("Failed to create local node record");
        if let Ok(fork_id) = storage.get_fork_id().await {
            local_node_record
                .set_fork_id(fork_id, &signer)
                .expect("Failed to set fork_id on local node record");
        }

        let mut discovery_server = Self {
            local_node: local_node.clone(),
            local_node_record,
            signer,
            udp_socket: Arc::new(udp_socket),
            store: storage.clone(),
            peer_table: peer_table.clone(),
            find_node_message: Self::random_message(&signer),
            initial_lookup_interval,
            next_lookup_is_enr: false,
        };

        info!(count = bootnodes.len(), "Adding bootnodes");

        for bootnode in &bootnodes {
            discovery_server.send_ping(bootnode).await?;
        }
        peer_table
            .new_contacts(bootnodes, local_node.node_id())
            .await?;

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
    ) -> Result<(), DiscoveryServerError> {
        // Ignore packets sent by ourselves
        if node_id(&sender_public_key) == self.local_node.node_id() {
            return Ok(());
        }
        match message {
            Message::Ping(ping_message) => {
                trace!(received = "Ping", msg = ?ping_message, from = %format!("{sender_public_key:#x}"));

                if is_msg_expired(ping_message.expiration) {
                    trace!("Ping expired, skipped");
                    return Ok(());
                }

                let node = Node::new(
                    from.ip().to_canonical(),
                    from.port(),
                    ping_message.from.tcp_port,
                    sender_public_key,
                );

                let _ = self.handle_ping(ping_message, hash, sender_public_key, node).await.inspect_err(|e| {
                    error!(sent = "Ping", to = %format!("{sender_public_key:#x}"), err = ?e, "Error handling message");
                });
            }
            Message::Pong(pong_message) => {
                trace!(received = "Pong", msg = ?pong_message, from = %format!("{:#x}", sender_public_key));

                let node_id = node_id(&sender_public_key);

                self.handle_pong(pong_message, node_id).await?;
            }
            Message::FindNode(find_node_message) => {
                trace!(received = "FindNode", msg = ?find_node_message, from = %format!("{:#x}", sender_public_key));

                if is_msg_expired(find_node_message.expiration) {
                    trace!("FindNode expired, skipped");
                    return Ok(());
                }

                self.handle_find_node(sender_public_key, find_node_message.target, from)
                    .await?;
            }
            Message::Neighbors(neighbors_message) => {
                trace!(received = "Neighbors", msg = ?neighbors_message, from = %format!("{sender_public_key:#x}"));

                if is_msg_expired(neighbors_message.expiration) {
                    trace!("Neighbors expired, skipping");
                    return Ok(());
                }

                self.handle_neighbors(neighbors_message).await?;
            }
            Message::ENRRequest(enrrequest_message) => {
                trace!(received = "ENRRequest", msg = ?enrrequest_message, from = %format!("{sender_public_key:#x}"));

                if is_msg_expired(enrrequest_message.expiration) {
                    trace!("ENRRequest expired, skipping");
                    return Ok(());
                }

                self.handle_enr_request(sender_public_key, from, hash)
                    .await?;
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
                self.handle_enr_response(sender_public_key, from, enrresponse_message)
                    .await?;
            }
        }
        Ok(())
    }

    /// Generate and store a FindNodeMessage with a random key. We then send the same message on Disovery lookup.
    /// We change this message every CHANGE_FIND_NODE_MESSAGE_INTERVAL.
    fn random_message(signer: &SecretKey) -> BytesMut {
        let expiration: u64 = get_msg_expiration_from_seconds(EXPIRATION_SECONDS);
        let random_priv_key = SecretKey::new(&mut OsRng);
        let random_pub_key = public_key_from_signing_key(&random_priv_key);
        let msg = Message::FindNode(FindNodeMessage::new(random_pub_key, expiration));
        let mut buf = BytesMut::new();
        msg.encode_with_header(&mut buf, signer);
        buf
    }

    async fn revalidate(&mut self) -> Result<(), DiscoveryServerError> {
        for contact in self
            .peer_table
            .get_contacts_to_revalidate(REVALIDATION_INTERVAL)
            .await?
        {
            self.send_ping(&contact.node).await?;
        }
        Ok(())
    }

    async fn lookup(&mut self) -> Result<(), DiscoveryServerError> {
        if let Some(contact) = self.peer_table.get_contact_for_lookup().await? {
            if let Err(e) = self
                .udp_socket
                .send_to(&self.find_node_message, &contact.node.udp_addr())
                .await
            {
                error!(sending = "FindNode", addr = ?&contact.node.udp_addr(), err=?e, "Error sending message");
                self.peer_table
                    .set_disposable(&contact.node.node_id())
                    .await?;
                METRICS.record_new_discarded_node();
            }

            self.peer_table
                .increment_find_node_sent(&contact.node.node_id())
                .await?;
        }
        Ok(())
    }

    async fn prune(&mut self) -> Result<(), DiscoveryServerError> {
        self.peer_table.prune().await?;
        Ok(())
    }

    async fn get_lookup_interval(&mut self) -> Duration {
        let peer_completion = self
            .peer_table
            .target_peers_completion()
            .await
            .unwrap_or_default();
        lookup_interval_function(
            peer_completion,
            self.initial_lookup_interval,
            LOOKUP_INTERVAL_MS,
        )
    }

    async fn enr_lookup(&mut self) -> Result<(), DiscoveryServerError> {
        if let Some(contact) = self.peer_table.get_contact_for_enr_lookup().await? {
            self.send_enr_request(&contact.node).await?;
        }
        Ok(())
    }

    async fn send_ping(&mut self, node: &Node) -> Result<(), DiscoveryServerError> {
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
        let enr_seq = self.local_node_record.seq;
        let ping = Message::Ping(PingMessage::new(from, to, expiration).with_enr_seq(enr_seq));
        let ping_hash = self.send_else_dispose(ping, node).await?;
        trace!(sent = "Ping", to = %format!("{:#x}", node.public_key));
        METRICS.record_ping_sent().await;
        self.peer_table
            .record_ping_sent(&node.node_id(), ping_hash)
            .await?;
        Ok(())
    }

    async fn send_pong(&self, ping_hash: H256, node: &Node) -> Result<(), DiscoveryServerError> {
        // TODO: Parametrize this expiration.
        let expiration: u64 = get_msg_expiration_from_seconds(EXPIRATION_SECONDS);

        let to = Endpoint {
            ip: node.ip,
            udp_port: node.udp_port,
            tcp_port: node.tcp_port,
        };

        let enr_seq = self.local_node_record.seq;

        let pong = Message::Pong(PongMessage::new(to, ping_hash, expiration).with_enr_seq(enr_seq));

        self.send(pong, node.udp_addr()).await?;

        trace!(sent = "Pong", to = %format!("{:#x}", node.public_key));

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

        trace!(sent = "Neighbors", to = %format!("{:#x}", node.public_key));

        Ok(())
    }

    async fn send_enr_request(&mut self, node: &Node) -> Result<(), DiscoveryServerError> {
        let expiration: u64 = get_msg_expiration_from_seconds(EXPIRATION_SECONDS);
        let enr_request = Message::ENRRequest(ENRRequestMessage { expiration });

        let enr_request_hash = self.send_else_dispose(enr_request, node).await?;

        self.peer_table
            .record_enr_request_sent(&node.node_id(), enr_request_hash)
            .await?;
        Ok(())
    }

    async fn send_enr_response(
        &self,
        request_hash: H256,
        from: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        let node_record = &self.local_node_record;

        let msg = Message::ENRResponse(ENRResponseMessage::new(request_hash, node_record.clone()));

        self.send(msg, from).await?;

        Ok(())
    }

    async fn handle_ping(
        &mut self,
        ping_message: PingMessage,
        hash: H256,
        sender_public_key: H512,
        node: Node,
    ) -> Result<(), DiscoveryServerError> {
        self.send_pong(hash, &node).await?;

        if self.peer_table.insert_if_new(&node).await.unwrap_or(false) {
            self.send_ping(&node).await?;
        } else {
            // If the contact has stale ENR then request the updated one.
            let node_id = node_id(&sender_public_key);
            let stored_enr_seq = self
                .peer_table
                .get_contact(node_id)
                .await?
                .and_then(|c| c.record)
                .map(|r| r.seq);

            let received_enr_seq = ping_message.enr_seq;

            if let (Some(received), Some(stored)) = (received_enr_seq, stored_enr_seq)
                && received > stored
            {
                self.send_enr_request(&node).await?;
            }
        }
        Ok(())
    }

    async fn handle_pong(
        &mut self,
        message: PongMessage,
        node_id: H256,
    ) -> Result<(), DiscoveryServerError> {
        let Some(contact) = self.peer_table.get_contact(node_id).await? else {
            return Ok(());
        };

        // If the contact doesn't exist then there is nothing to record.
        // So we do it after making sure that the contact exists.
        self.peer_table
            .record_pong_received(&node_id, message.ping_hash)
            .await?;

        // If the contact has stale ENR then request the updated one.
        let stored_enr_seq = contact.record.map(|r| r.seq);
        let received_enr_seq = message.enr_seq;
        if let (Some(received), Some(stored)) = (received_enr_seq, stored_enr_seq)
            && received > stored
        {
            self.send_enr_request(&contact.node).await?;
        }

        Ok(())
    }

    async fn handle_find_node(
        &mut self,
        sender_public_key: H512,
        target: H512,
        from: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        let sender_id = node_id(&sender_public_key);
        if let Ok(contact) = self
            .validate_contact(sender_public_key, sender_id, from, "FindNode")
            .await
        {
            // According to https://github.com/ethereum/devp2p/blob/master/discv4.md#findnode-packet-0x03
            // reply closest 16 nodes to target
            let target_id = node_id(&target);
            let neighbors = self.peer_table.get_closest_nodes(&target_id).await?;

            // A single node encodes to at most 89B, so 8 of them are at most 712B plus
            // recursive length and expiration time, well within bound of 1280B per packet.
            // Sending all in one packet would exceed bounds with the nodes only, weighing
            // up to 1424B.
            for chunk in neighbors.chunks(8) {
                let _ = self.send_neighbors(chunk.to_vec(), &contact.node).await;
            }
        }
        Ok(())
    }

    async fn handle_neighbors(
        &mut self,
        neighbors_message: NeighborsMessage,
    ) -> Result<(), DiscoveryServerError> {
        // TODO(#3746): check that we requested neighbors from the node
        let nodes = neighbors_message.nodes.clone();
        self.peer_table
            .new_contacts(nodes, self.local_node.node_id())
            .await?;
        for node in neighbors_message.nodes {
            self.send_ping(&node).await?;
        }
        Ok(())
    }

    async fn handle_enr_request(
        &mut self,
        sender_public_key: H512,
        from: SocketAddr,
        hash: H256,
    ) -> Result<(), DiscoveryServerError> {
        let node_id = node_id(&sender_public_key);

        if self
            .validate_contact(sender_public_key, node_id, from, "ENRRequest")
            .await
            .is_err()
        {
            return Ok(());
        }

        if self.send_enr_response(hash, from).await.is_err() {
            return Ok(());
        }

        self.peer_table.knows_us(&node_id).await?;
        Ok(())
    }

    async fn handle_enr_response(
        &mut self,
        sender_public_key: H512,
        from: SocketAddr,
        enr_response_message: ENRResponseMessage,
    ) -> Result<(), DiscoveryServerError> {
        let node_id = node_id(&sender_public_key);

        if self
            .validate_enr_response(sender_public_key, node_id, from)
            .await
            .is_err()
        {
            return Ok(());
        }

        self.peer_table
            .record_enr_response_received(
                &node_id,
                enr_response_message.request_hash,
                enr_response_message.node_record.clone(),
            )
            .await?;

        self.validate_enr_fork_id(node_id, sender_public_key, enr_response_message.node_record)
            .await?;

        Ok(())
    }

    /// Validates the fork id of the given ENR is valid, saving it to the peer_table.
    async fn validate_enr_fork_id(
        &mut self,
        node_id: H256,
        sender_public_key: H512,
        node_record: NodeRecord,
    ) -> Result<(), DiscoveryServerError> {
        let pairs = node_record.decode_pairs();

        let Some(remote_fork_id) = pairs.eth else {
            self.peer_table
                .set_is_fork_id_valid(&node_id, false)
                .await?;
            debug!(received = "ENRResponse", from = %format!("{sender_public_key:#x}"), "missing fork id in ENR response, skipping");
            return Ok(());
        };

        let chain_config = self.store.get_chain_config();
        let genesis_header = self
            .store
            .get_block_header(0)?
            .ok_or(DiscoveryServerError::InvalidContact)?;
        let latest_block_number = self.store.get_latest_block_number().await?;
        let latest_block_header = self
            .store
            .get_block_header(latest_block_number)?
            .ok_or(DiscoveryServerError::InvalidContact)?;

        let local_fork_id = ForkId::new(
            chain_config,
            genesis_header.clone(),
            latest_block_header.timestamp,
            latest_block_number,
        );

        if !backend::is_fork_id_valid(&self.store, &remote_fork_id).await? {
            self.peer_table
                .set_is_fork_id_valid(&node_id, false)
                .await?;
            debug!(received = "ENRResponse", from = %format!("{sender_public_key:#x}"), local_fork_id=%local_fork_id, remote_fork_id=%remote_fork_id, "fork id mismatch in ENR response, skipping");
            return Ok(());
        }

        debug!(received = "ENRResponse", from = %format!("{sender_public_key:#x}"), local_fork_id=%local_fork_id, remote_fork_id=%remote_fork_id, "valid fork id in ENR found");
        self.peer_table.set_is_fork_id_valid(&node_id, true).await?;

        Ok(())
    }

    async fn validate_contact(
        &mut self,
        sender_public_key: H512,
        node_id: H256,
        from: SocketAddr,
        message_type: &str,
    ) -> Result<Contact, DiscoveryServerError> {
        match self
            .peer_table
            .validate_contact(&node_id, from.ip())
            .await?
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
            PeerTableOutMessage::Contact(contact) => Ok(*contact),
            _ => unreachable!(),
        }
    }

    async fn validate_enr_response(
        &mut self,
        sender_public_key: H512,
        node_id: H256,
        from: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        let contact = self
            .validate_contact(sender_public_key, node_id, from, "ENRResponse")
            .await?;
        if !contact.has_pending_enr_request() {
            debug!(received = "ENRResponse", from = %format!("{sender_public_key:#x}"), "unsolicited message received, skipping");
            return Err(DiscoveryServerError::InvalidContact);
        }
        Ok(())
    }

    async fn send(
        &self,
        message: Message,
        addr: SocketAddr,
    ) -> Result<usize, DiscoveryServerError> {
        let mut buf = BytesMut::new();
        message.encode_with_header(&mut buf, &self.signer);
        Ok(self.udp_socket.send_to(&buf, addr).await.inspect_err(
            |e| error!(sending = ?message, addr = ?addr, err=?e, "Error sending message"),
        )?)
    }

    async fn send_else_dispose(
        &mut self,
        message: Message,
        node: &Node,
    ) -> Result<H256, DiscoveryServerError> {
        let mut buf = BytesMut::new();
        message.encode_with_header(&mut buf, &self.signer);
        let message_hash: [u8; 32] = buf[..32]
            .try_into()
            .expect("first 32 bytes are the message hash");
        if let Err(e) = self.udp_socket.send_to(&buf, node.udp_addr()).await {
            error!(sending = ?message, addr = ?node.udp_addr(), to = ?node.node_id(), err=?e, "Error sending message");
            self.peer_table.set_disposable(&node.node_id()).await?;
            METRICS.record_new_discarded_node();
        }
        Ok(H256::from(message_hash))
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
        let stream = UdpFramed::new(self.udp_socket.clone(), Discv4Codec::new(self.signer));

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
        send_interval(
            CHANGE_FIND_NODE_MESSAGE_INTERVAL,
            handle.clone(),
            InMessage::ChangeFindNodeMessage,
        );
        let _ = handle.clone().cast(InMessage::Lookup).await;
        send_message_on(handle.clone(), tokio::signal::ctrl_c(), InMessage::Shutdown);

        Ok(Success(self))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            Self::CastMsg::Message(message) => {
                let _ = self
                    .handle_message(*message)
                    .await
                    .inspect_err(|e| error!(err=?e, "Error Handling Discovery message"));
            }
            Self::CastMsg::Revalidate => {
                trace!(received = "Revalidate");
                let _ = self
                    .revalidate()
                    .await
                    .inspect_err(|e| error!(err=?e, "Error revalidating discovered peers"));
            }
            Self::CastMsg::Lookup => {
                // Check if target peers are met — use slow maintenance rate
                let peer_completion = self
                    .peer_table
                    .target_peers_completion()
                    .await
                    .unwrap_or_default();

                let interval = if peer_completion >= 1.0 {
                    MAINTENANCE_LOOKUP_INTERVAL
                } else {
                    self.get_lookup_interval().await
                };

                // Alternate between regular lookup and ENR lookup
                if self.next_lookup_is_enr {
                    trace!(received = "EnrLookup");
                    let _ = self
                        .enr_lookup()
                        .await
                        .inspect_err(|e| error!(err=?e, "Error performing Discovery ENR lookup"));
                } else {
                    trace!(received = "Lookup");
                    let _ = self
                        .lookup()
                        .await
                        .inspect_err(|e| error!(err=?e, "Error performing Discovery lookup"));
                }
                self.next_lookup_is_enr = !self.next_lookup_is_enr;

                send_after(interval, handle.clone(), Self::CastMsg::Lookup);
            }
            Self::CastMsg::Prune => {
                trace!(received = "Prune");
                let _ = self
                    .prune()
                    .await
                    .inspect_err(|e| error!(err=?e, "Error Pruning peer table"));
            }
            Self::CastMsg::ChangeFindNodeMessage => {
                self.find_node_message = Self::random_message(&self.signer);
            }
            Self::CastMsg::Shutdown => return CastResponse::Stop,
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

pub fn lookup_interval_function(progress: f64, lower_limit: f64, upper_limit: f64) -> Duration {
    // Smooth progression curve
    // See https://easings.net/#easeInOutCubic
    let ease_in_out_cubic = if progress < 0.5 {
        4.0 * progress.powf(3.0)
    } else {
        1.0 - ((-2.0 * progress + 2.0).powf(3.0)) / 2.0
    };
    Duration::from_micros(
        // Use `progress` here instead of `ease_in_out_cubic` for a linear function.
        (1000f64 * (ease_in_out_cubic * (upper_limit - lower_limit) + lower_limit)).round() as u64,
    )
}

// TODO: Reimplement tests removed during snap sync refactor
//       https://github.com/lambdaclass/ethrex/issues/4423
