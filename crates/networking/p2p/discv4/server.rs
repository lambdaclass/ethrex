use crate::{
    backend,
    discv4::messages::{
        ENRRequestMessage, ENRResponseMessage, FindNodeMessage, Message, NeighborsMessage, Packet,
        PacketDecodeErr, PingMessage, PongMessage,
    },
    metrics::METRICS,
    peer_table::{
        Contact, ContactValidation, DiscoveryProtocol, PeerTable, PeerTableServerProtocol as _,
    },
    types::{Endpoint, INITIAL_ENR_SEQ, Node, NodeRecord},
    utils::{
        get_msg_expiration_from_seconds, is_msg_expired, node_id, public_key_from_signing_key,
    },
};
use bytes::{Bytes, BytesMut};
use ethrex_common::{H256, H512, types::ForkId};
use ethrex_storage::{Store, error::StoreError};
use rand::rngs::OsRng;
use secp256k1::SecretKey;
use spawned_concurrency::{
    actor,
    error::ActorError,
    protocol,
    tasks::{
        Actor, ActorRef, ActorStart as _, Context, Handler, send_after, send_interval,
        send_message_on,
    },
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::net::UdpSocket;
use tracing::{debug, error, info, trace};

const EXPIRATION_SECONDS: u64 = 20;
/// Interval between revalidation checks. Each check pings one random stale
/// contact, so this controls the maximum revalidation ping rate (~1/sec).
const REVALIDATION_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// Interval between revalidations.
const REVALIDATION_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60); // 12 hours,
/// The initial interval between peer lookups, until the number of peers reaches
/// [target_peers](DiscoverySideCarState::target_peers), or the number of
/// contacts reaches [target_contacts](DiscoverySideCarState::target_contacts).
pub const INITIAL_LOOKUP_INTERVAL_MS: f64 = 100.0; // 10 per second
pub const LOOKUP_INTERVAL_MS: f64 = 600.0; // 100 per minute
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
    PeerTable(#[from] ActorError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[protocol]
pub trait Discv4ServerProtocol: Send + Sync {
    fn recv_message(&self, message: Box<Discv4Message>) -> Result<(), ActorError>;
    fn revalidate(&self) -> Result<(), ActorError>;
    fn lookup(&self) -> Result<(), ActorError>;
    fn enr_lookup(&self) -> Result<(), ActorError>;
    fn prune(&self) -> Result<(), ActorError>;
    fn change_find_node_message(&self) -> Result<(), ActorError>;
    fn shutdown(&self) -> Result<(), ActorError>;
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
    /// Tracks pending FindNode requests by node_id -> sent_at.
    /// Used to reject unsolicited Neighbors responses.
    pending_find_node: HashMap<H256, Instant>,
}

#[actor(protocol = Discv4ServerProtocol)]
impl DiscoveryServer {
    /// Spawn the discv4 discovery server.
    ///
    /// The server receives packets from the multiplexer via actor sends.
    /// The `udp_socket` is shared with the multiplexer and used for sending only.
    pub async fn spawn(
        storage: Store,
        local_node: Node,
        signer: SecretKey,
        udp_socket: Arc<UdpSocket>,
        peer_table: PeerTable,
        bootnodes: Vec<Node>,
        initial_lookup_interval: f64,
    ) -> Result<ActorRef<Self>, DiscoveryServerError> {
        info!(protocol = "discv4", "Starting discovery server");

        let mut local_node_record = NodeRecord::from_node(&local_node, INITIAL_ENR_SEQ, &signer)
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
            udp_socket,
            store: storage.clone(),
            peer_table: peer_table.clone(),
            find_node_message: Self::random_message(&signer),
            initial_lookup_interval,
            pending_find_node: HashMap::new(),
        };

        info!(
            protocol = "discv4",
            count = bootnodes.len(),
            "Adding bootnodes"
        );

        peer_table.new_contacts(
            bootnodes.clone(),
            local_node.node_id(),
            DiscoveryProtocol::Discv4,
        )?;

        for bootnode in &bootnodes {
            discovery_server.send_ping(bootnode).await?;
        }

        Ok(discovery_server.start())
    }

    #[started]
    async fn started(&mut self, ctx: &Context<Self>) {
        send_interval(
            REVALIDATION_CHECK_INTERVAL,
            ctx.clone(),
            discv4_server_protocol::Revalidate,
        );
        send_interval(PRUNE_INTERVAL, ctx.clone(), discv4_server_protocol::Prune);
        send_interval(
            CHANGE_FIND_NODE_MESSAGE_INTERVAL,
            ctx.clone(),
            discv4_server_protocol::ChangeFindNodeMessage,
        );
        let _ = ctx.send(discv4_server_protocol::Lookup);
        let _ = ctx.send(discv4_server_protocol::EnrLookup);
        send_message_on(
            ctx.clone(),
            tokio::signal::ctrl_c(),
            discv4_server_protocol::Shutdown,
        );
    }

    #[send_handler]
    async fn handle_recv_message(
        &mut self,
        msg: discv4_server_protocol::RecvMessage,
        _ctx: &Context<Self>,
    ) {
        let _ = self.process_message(*msg.message).await.inspect_err(
            |e| error!(protocol = "discv4", err=?e, "Error Handling Discovery message"),
        );
    }

    #[send_handler]
    async fn handle_revalidate(
        &mut self,
        _msg: discv4_server_protocol::Revalidate,
        _ctx: &Context<Self>,
    ) {
        trace!(protocol = "discv4", received = "Revalidate");
        let _ = self.revalidate_peers().await.inspect_err(
            |e| error!(protocol = "discv4", err=?e, "Error revalidating discovered peers"),
        );
    }

    #[send_handler]
    async fn handle_lookup(&mut self, _msg: discv4_server_protocol::Lookup, ctx: &Context<Self>) {
        trace!(protocol = "discv4", received = "Lookup");
        let _ = self.do_lookup().await.inspect_err(
            |e| error!(protocol = "discv4", err=?e, "Error performing Discovery lookup"),
        );

        let interval = self.get_lookup_interval().await;
        send_after(interval, ctx.clone(), discv4_server_protocol::Lookup);
    }

    #[send_handler]
    async fn handle_enr_lookup(
        &mut self,
        _msg: discv4_server_protocol::EnrLookup,
        ctx: &Context<Self>,
    ) {
        trace!(protocol = "discv4", received = "EnrLookup");
        let _ = self.do_enr_lookup().await.inspect_err(
            |e| error!(protocol = "discv4", err=?e, "Error performing Discovery lookup"),
        );

        let interval = self.get_lookup_interval().await;
        send_after(interval, ctx.clone(), discv4_server_protocol::EnrLookup);
    }

    #[send_handler]
    async fn handle_prune(&mut self, _msg: discv4_server_protocol::Prune, _ctx: &Context<Self>) {
        trace!(protocol = "discv4", received = "Prune");
        let _ = self
            .prune()
            .await
            .inspect_err(|e| error!(protocol = "discv4", err=?e, "Error Pruning peer table"));
    }

    #[send_handler]
    async fn handle_change_find_node_message(
        &mut self,
        _msg: discv4_server_protocol::ChangeFindNodeMessage,
        _ctx: &Context<Self>,
    ) {
        self.find_node_message = Self::random_message(&self.signer);
    }

    #[send_handler]
    async fn handle_shutdown(
        &mut self,
        _msg: discv4_server_protocol::Shutdown,
        ctx: &Context<Self>,
    ) {
        ctx.stop();
    }

    async fn process_message(
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
        #[cfg(feature = "metrics")]
        {
            use ethrex_metrics::p2p::METRICS_P2P;
            METRICS_P2P.inc_discv4_incoming(message.metric_label());
        }
        match message {
            Message::Ping(ping_message) => {
                trace!(protocol = "discv4", received = "Ping", msg = ?ping_message, from = %format!("{sender_public_key:#x}"));

                if is_msg_expired(ping_message.expiration) {
                    trace!(protocol = "discv4", "Ping expired, skipped");
                    return Ok(());
                }

                let node = Node::new(
                    from.ip().to_canonical(),
                    from.port(),
                    ping_message.from.tcp_port,
                    sender_public_key,
                );

                let _ = self.handle_ping(ping_message, hash, sender_public_key, node).await.inspect_err(|e| {
                    error!(protocol = "discv4", sent = "Ping", to = %format!("{sender_public_key:#x}"), err = ?e, "Error handling message");
                });
            }
            Message::Pong(pong_message) => {
                trace!(protocol = "discv4", received = "Pong", msg = ?pong_message, from = %format!("{:#x}", sender_public_key));

                let node_id = node_id(&sender_public_key);

                self.handle_pong(pong_message, node_id).await?;
            }
            Message::FindNode(find_node_message) => {
                trace!(protocol = "discv4", received = "FindNode", msg = ?find_node_message, from = %format!("{:#x}", sender_public_key));

                if is_msg_expired(find_node_message.expiration) {
                    trace!(protocol = "discv4", "FindNode expired, skipped");
                    return Ok(());
                }

                self.handle_find_node(sender_public_key, find_node_message.target, from)
                    .await?;
            }
            Message::Neighbors(neighbors_message) => {
                trace!(protocol = "discv4", received = "Neighbors", msg = ?neighbors_message, from = %format!("{sender_public_key:#x}"));

                if is_msg_expired(neighbors_message.expiration) {
                    trace!(protocol = "discv4", "Neighbors expired, skipping");
                    return Ok(());
                }

                self.handle_neighbors(neighbors_message, sender_public_key)
                    .await?;
            }
            Message::ENRRequest(enrrequest_message) => {
                trace!(protocol = "discv4", received = "ENRRequest", msg = ?enrrequest_message, from = %format!("{sender_public_key:#x}"));

                if is_msg_expired(enrrequest_message.expiration) {
                    trace!(protocol = "discv4", "ENRRequest expired, skipping");
                    return Ok(());
                }

                self.handle_enr_request(sender_public_key, from, hash)
                    .await?;
            }
            Message::ENRResponse(enrresponse_message) => {
                trace!(protocol = "discv4", received = "ENRResponse", msg = ?enrresponse_message, from = %format!("{sender_public_key:#x}"));
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

    async fn revalidate_peers(&mut self) -> Result<(), DiscoveryServerError> {
        if let Some(contact) = self
            .peer_table
            .get_contact_to_revalidate(REVALIDATION_INTERVAL, DiscoveryProtocol::Discv4)
            .await?
        {
            self.send_ping(&contact.node).await?;
        }
        Ok(())
    }

    async fn do_lookup(&mut self) -> Result<(), DiscoveryServerError> {
        if let Some(contact) = self
            .peer_table
            .get_contact_for_lookup(DiscoveryProtocol::Discv4)
            .await?
        {
            if let Err(e) = self
                .udp_socket
                .send_to(&self.find_node_message, &contact.node.udp_addr())
                .await
            {
                error!(protocol = "discv4", sending = "FindNode", addr = ?&contact.node.udp_addr(), err=?e, "Error sending message");
                self.peer_table.set_disposable(contact.node.node_id())?;
                METRICS.record_new_discarded_node();
            } else {
                #[cfg(feature = "metrics")]
                {
                    use ethrex_metrics::p2p::METRICS_P2P;
                    METRICS_P2P.inc_discv4_outgoing("FindNode");
                }
                self.pending_find_node
                    .insert(contact.node.node_id(), Instant::now());
            }

            self.peer_table
                .increment_find_node_sent(contact.node.node_id())?;
        }
        Ok(())
    }

    async fn prune(&mut self) -> Result<(), DiscoveryServerError> {
        self.peer_table.prune_table()?;
        // Clean up expired pending FindNode entries
        let expiration = Duration::from_secs(EXPIRATION_SECONDS);
        self.pending_find_node
            .retain(|_, sent_at| sent_at.elapsed() < expiration);
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

    async fn do_enr_lookup(&mut self) -> Result<(), DiscoveryServerError> {
        if let Some(contact) = self.peer_table.get_contact_for_enr_lookup().await? {
            self.send_enr_request(&contact.node).await?;
        }
        Ok(())
    }

    async fn send_ping(&mut self, node: &Node) -> Result<(), DiscoveryServerError> {
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
        trace!(protocol = "discv4", sent = "Ping", to = %format!("{:#x}", node.public_key));
        METRICS.record_ping_sent().await;
        let ping_id = Bytes::copy_from_slice(ping_hash.as_bytes());
        self.peer_table.record_ping_sent(node.node_id(), ping_id)?;
        Ok(())
    }

    async fn send_pong(&self, ping_hash: H256, node: &Node) -> Result<(), DiscoveryServerError> {
        let expiration: u64 = get_msg_expiration_from_seconds(EXPIRATION_SECONDS);

        let to = Endpoint {
            ip: node.ip,
            udp_port: node.udp_port,
            tcp_port: node.tcp_port,
        };

        let enr_seq = self.local_node_record.seq;

        let pong = Message::Pong(PongMessage::new(to, ping_hash, expiration).with_enr_seq(enr_seq));

        self.send(pong, node.udp_addr()).await?;

        trace!(protocol = "discv4", sent = "Pong", to = %format!("{:#x}", node.public_key));

        Ok(())
    }

    async fn send_neighbors(
        &self,
        neighbors: Vec<Node>,
        node: &Node,
    ) -> Result<(), DiscoveryServerError> {
        let expiration: u64 = get_msg_expiration_from_seconds(EXPIRATION_SECONDS);

        let msg = Message::Neighbors(NeighborsMessage::new(neighbors, expiration));

        self.send(msg, node.udp_addr()).await?;

        trace!(protocol = "discv4", sent = "Neighbors", to = %format!("{:#x}", node.public_key));

        Ok(())
    }

    async fn send_enr_request(&mut self, node: &Node) -> Result<(), DiscoveryServerError> {
        let expiration: u64 = get_msg_expiration_from_seconds(EXPIRATION_SECONDS);
        let enr_request = Message::ENRRequest(ENRRequestMessage { expiration });

        let enr_request_hash = self.send_else_dispose(enr_request, node).await?;

        self.peer_table
            .record_enr_request_sent(node.node_id(), enr_request_hash)?;
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

        if self
            .peer_table
            .insert_if_new(node.clone(), DiscoveryProtocol::Discv4)
            .await
            .unwrap_or(false)
        {
            self.send_ping(&node).await?;
        } else {
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

        let ping_id = Bytes::copy_from_slice(message.ping_hash.as_bytes());
        self.peer_table.record_pong_received(node_id, ping_id)?;

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
            let target_id = node_id(&target);
            let neighbors = self.peer_table.get_closest_nodes(target_id).await?;

            for chunk in neighbors.chunks(8) {
                let _ = self.send_neighbors(chunk.to_vec(), &contact.node).await;
            }
        }
        Ok(())
    }

    async fn handle_neighbors(
        &mut self,
        neighbors_message: NeighborsMessage,
        sender_public_key: H512,
    ) -> Result<(), DiscoveryServerError> {
        let sender_id = node_id(&sender_public_key);
        let expiration = Duration::from_secs(EXPIRATION_SECONDS);

        // Only accept Neighbors from peers we sent a FindNode to.
        // This prevents unsolicited Neighbors from injecting contacts
        // into our peer table. We don't remove the entry on first
        // response because Neighbors can be split across multiple
        // UDP packets (up to 8 nodes each).
        match self.pending_find_node.get(&sender_id) {
            Some(sent_at) if sent_at.elapsed() < expiration => {}
            _ => {
                trace!(
                    protocol = "discv4",
                    from = %format!("{sender_public_key:#x}"),
                    "Dropping unsolicited Neighbors (no pending FindNode)"
                );
                return Ok(());
            }
        }

        let nodes = neighbors_message.nodes;
        self.peer_table.new_contacts(
            nodes,
            self.local_node.node_id(),
            DiscoveryProtocol::Discv4,
        )?;
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

        self.peer_table.mark_knows_us(node_id)?;
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

        self.peer_table.record_enr_response_received(
            node_id,
            enr_response_message.request_hash,
            enr_response_message.node_record.clone(),
        )?;

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
        let node_fork_id = node_record.get_fork_id().cloned();

        let Some(remote_fork_id) = node_fork_id else {
            self.peer_table.set_is_fork_id_valid(node_id, false)?;
            debug!(protocol = "discv4", received = "ENRResponse", from = %format!("{sender_public_key:#x}"), "missing fork id in ENR response, skipping");
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
            self.peer_table.set_is_fork_id_valid(node_id, false)?;
            debug!(protocol = "discv4", received = "ENRResponse", from = %format!("{sender_public_key:#x}"), local_fork_id=%local_fork_id, remote_fork_id=%remote_fork_id, "fork id mismatch in ENR response, skipping");
            return Ok(());
        }

        debug!(protocol = "discv4", received = "ENRResponse", from = %format!("{sender_public_key:#x}"), local_fork_id=%local_fork_id, remote_fork_id=%remote_fork_id, "valid fork id in ENR found");
        self.peer_table.set_is_fork_id_valid(node_id, true)?;

        Ok(())
    }

    async fn validate_contact(
        &mut self,
        sender_public_key: H512,
        node_id: H256,
        from: SocketAddr,
        message_type: &str,
    ) -> Result<Contact, DiscoveryServerError> {
        match self.peer_table.validate_contact(node_id, from.ip()).await? {
            ContactValidation::UnknownContact => {
                debug!(protocol = "discv4", received = message_type, to = %format!("{sender_public_key:#x}"), "Unknown contact, skipping");
                Err(DiscoveryServerError::InvalidContact)
            }
            ContactValidation::InvalidContact => {
                debug!(protocol = "discv4", received = message_type, to = %format!("{sender_public_key:#x}"), "Contact not validated, skipping");
                Err(DiscoveryServerError::InvalidContact)
            }
            ContactValidation::IpMismatch => {
                debug!(protocol = "discv4", received = message_type, to = %format!("{sender_public_key:#x}"), "IP address mismatch, skipping");
                Err(DiscoveryServerError::InvalidContact)
            }
            ContactValidation::Valid(contact) => Ok(*contact),
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
            debug!(protocol = "discv4", received = "ENRResponse", from = %format!("{sender_public_key:#x}"), "unsolicited message received, skipping");
            return Err(DiscoveryServerError::InvalidContact);
        }
        Ok(())
    }

    async fn send(
        &self,
        message: Message,
        addr: SocketAddr,
    ) -> Result<usize, DiscoveryServerError> {
        #[cfg(feature = "metrics")]
        {
            use ethrex_metrics::p2p::METRICS_P2P;
            METRICS_P2P.inc_discv4_outgoing(message.metric_label());
        }
        let mut buf = BytesMut::new();
        message.encode_with_header(&mut buf, &self.signer);
        Ok(self.udp_socket.send_to(&buf, addr).await.inspect_err(
            |e| error!(protocol = "discv4", sending = ?message, addr = ?addr, err=?e, "Error sending message"),
        )?)
    }

    async fn send_else_dispose(
        &mut self,
        message: Message,
        node: &Node,
    ) -> Result<H256, DiscoveryServerError> {
        #[cfg(feature = "metrics")]
        {
            use ethrex_metrics::p2p::METRICS_P2P;
            METRICS_P2P.inc_discv4_outgoing(message.metric_label());
        }
        let mut buf = BytesMut::new();
        message.encode_with_header(&mut buf, &self.signer);
        let message_hash: [u8; 32] = buf[..32]
            .try_into()
            .expect("first 32 bytes are the message hash");
        if let Err(e) = self.udp_socket.send_to(&buf, node.udp_addr()).await {
            error!(protocol = "discv4", sending = ?message, addr = ?node.udp_addr(), to = ?node.node_id(), err=?e, "Error sending message");
            self.peer_table.set_disposable(node.node_id())?;
            METRICS.record_new_discarded_node();
            return Err(e.into());
        }
        Ok(H256::from(message_hash))
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

pub use crate::discovery::lookup_interval_function;

// TODO: Reimplement tests removed during snap sync refactor
//       https://github.com/lambdaclass/ethrex/issues/4423
