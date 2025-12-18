use crate::{
    discv5::{
        codec::Discv5Codec,
        messages::{
            FindNodeMessage, Message, NodesMessage, Ordinary, Packet, PacketCodecError,
            PingMessage, PongMessage,
        },
    },
    metrics::METRICS,
    peer_table::{Contact, OutMessage as PeerTableOutMessage, PeerTable, PeerTableError},
    types::{Endpoint, Node, NodeRecord},
    utils::{get_msg_expiration_from_seconds, public_key_from_signing_key},
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
pub const INITIAL_LOOKUP_INTERVAL_MS: f64 = 100.0; // 10 per second
pub const LOOKUP_INTERVAL_MS: f64 = 600.0; // 100 per minute
const CHANGE_FIND_NODE_MESSAGE_INTERVAL: Duration = Duration::from_secs(5);
const PRUNE_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryServerError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("Failed to decode packet")]
    InvalidPacket(#[from] PacketCodecError),
    #[error("Failed to send message")]
    MessageSendFailure(PacketCodecError),
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
    Message(Box<Discv5Message>),
    Revalidate,
    Lookup,
    Prune,
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
    node_id: H256,
    udp_socket: Arc<UdpSocket>,
    store: Store,
    peer_table: PeerTable,
    initial_lookup_interval: f64,
}

impl DiscoveryServer {
    pub async fn spawn(
        storage: Store,
        local_node: Node,
        signer: SecretKey,
        udp_socket: Arc<UdpSocket>,
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
            node_id: local_node.node_id(),
            udp_socket,
            store: storage.clone(),
            peer_table: peer_table.clone(),
            initial_lookup_interval,
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
        Discv5Message { from, packet }: Discv5Message,
    ) -> Result<(), DiscoveryServerError> {
        trace!(msg = ?packet, address= ?from, "Discv5 message received");
        match packet {
            Packet::Ordinary(ordinary) => todo!(),
            Packet::WhoAreYou(who_are_you) => todo!(),
            Packet::Handshake(handshake) => todo!(),
        }
        Ok(())
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
                .send(&Message::FindNode(rand::random()), &contact.node)
                .await
            {
                error!(sending = "FindNode", addr = ?&contact.node.udp_addr(), err=?e, "Error sending message");
                self.peer_table
                    .set_disposable(&contact.node.node_id())
                    .await?;
                METRICS.record_new_discarded_node().await;
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

    async fn send_find_node(&mut self, node: &Node) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    async fn send_ping(&mut self, node: &Node) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    async fn send_pong(&self, ping_hash: H256, node: &Node) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    async fn send_nodes(
        &self,
        neighbors: Vec<Node>,
        node: &Node,
    ) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    async fn handle_ping(
        &mut self,
        ping_message: PingMessage,
        hash: H256,
        sender_public_key: H512,
        node: Node,
    ) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    async fn handle_pong(
        &mut self,
        message: PongMessage,
        node_id: H256,
    ) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    async fn handle_find_node(
        &mut self,
        sender_public_key: H512,
        target: H512,
        from: SocketAddr,
    ) -> Result<(), DiscoveryServerError> {
        // TODO
        Ok(())
    }

    async fn handle_nodes(
        &mut self,
        nodes_message: NodesMessage,
    ) -> Result<(), DiscoveryServerError> {
        // TODO
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

        if !local_fork_id.is_valid(
            remote_fork_id.clone(),
            latest_block_number,
            latest_block_header.timestamp,
            chain_config,
            genesis_header,
        ) {
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

    async fn send(&self, message: &Message, node: &Node) -> Result<usize, DiscoveryServerError> {
        let packet = Packet::Ordinary(Ordinary {
            src_id: self.node_id,
            message: message.clone(),
        });
        let mut buf = BytesMut::new();
        packet
            .encode(&mut buf, 0, &[1; 12], &node.node_id(), &[0; 16])
            .unwrap();
        self.send_encoded(message, &buf, &node).await
    }

    async fn send_encoded(
        &self,
        message: &Message,
        buf: &BytesMut,
        node: &Node,
    ) -> Result<usize, DiscoveryServerError> {
        let addr = node.udp_addr();
        let size = self.udp_socket.send_to(&buf, &addr).await.inspect_err(
            |e| error!(sending = ?message, addr = ?addr, err=?e, "Error sending message"),
        )?;
        trace!(msg = %message, node = %node.public_key, address= %addr, "Discv5 message sent");
        Ok(size)
    }

    async fn send_else_dispose(
        &mut self,
        message: Message,
        node: &Node,
    ) -> Result<H256, DiscoveryServerError> {
        let mut buf = BytesMut::new();
        message.encode(&mut buf);
        let message_hash: [u8; 32] = buf[..32]
            .try_into()
            .expect("first 32 bytes are the message hash");
        if let Err(e) = self.udp_socket.send_to(&buf, node.udp_addr()).await {
            error!(sending = ?message, addr = ?node.udp_addr(), to = ?node.node_id(), err=?e, "Error sending message");
            self.peer_table.set_disposable(&node.node_id()).await?;
            METRICS.record_new_discarded_node().await;
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
        let stream = UdpFramed::new(self.udp_socket.clone(), Discv5Codec::new(self.node_id));

        spawn_listener(
            handle.clone(),
            stream.filter_map(|result| async move {
                match result {
                    Ok((msg, addr)) => {
                        Some(InMessage::Message(Box::new(Discv5Message::from(msg, addr))))
                    }
                    Err(e) => {
                        debug!(error=?e, "Error receiving Discv5 message");
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
                trace!(received = "Lookup");
                let _ = self
                    .lookup()
                    .await
                    .inspect_err(|e| error!(err=?e, "Error performing Discovery lookup"));

                let interval = self.get_lookup_interval().await;
                send_after(interval, handle.clone(), Self::CastMsg::Lookup);
            }
            Self::CastMsg::Prune => {
                trace!(received = "Prune");
                let _ = self
                    .prune()
                    .await
                    .inspect_err(|e| error!(err=?e, "Error Pruning peer table"));
            }
            Self::CastMsg::Shutdown => return CastResponse::Stop,
        }
        CastResponse::NoReply
    }
}

#[derive(Debug, Clone)]
pub struct Discv5Message {
    from: SocketAddr,
    packet: Packet,
}

impl Discv5Message {
    pub fn from(packet: Packet, from: SocketAddr) -> Self {
        Self { from, packet }
    }
}

pub fn lookup_interval_function(progress: f64, lower_limit: f64, upper_limit: f64) -> Duration {
    Duration::from_secs(5)
    // // Smooth progression curve
    // // See https://easings.net/#easeInOutCubic
    // let ease_in_out_cubic = if progress < 0.5 {
    //     4.0 * progress.powf(3.0)
    // } else {
    //     1.0 - ((-2.0 * progress + 2.0).powf(3.0)) / 2.0
    // };
    // Duration::from_micros(
    //     // Use `progress` here instead of `ease_in_out_cubic` for a linear function.
    //     (1000f64 * (ease_in_out_cubic * (upper_limit - lower_limit) + lower_limit)).round() as u64,
    // )
}
